#!/usr/bin/env bash
# gemma-uncensored — droplet installer. Runs as root on a fresh Ubuntu 24.04 Droplet.
#
# Installs llama.cpp (CPU build), downloads Gemma 4 26B-A4B Ultra Uncensored (Heretic)
# in the IQ4_XS quant (13.9 GB), generates two API keys, and starts an OpenAI-compatible
# server as a systemd service. Safe to re-run: it skips the build and the download when
# they are already complete.
#
# Knobs (environment variables, all optional):
#   GEMMA_CTX      context window in tokens     default 32768
#   GEMMA_PORT     listen port                  default 8080
#   GEMMA_BIND     listen address               default 0.0.0.0 (public; keys required)
#                  set 127.0.0.1 to keep it private and reach it over an SSH tunnel
#   GEMMA_THREADS  CPU threads                  default: all cores
#
# Progress when run from cloud-init:  tail -f /var/log/gemma-install.log
# When finished:                       cat /root/GEMMA-READY.txt
set -euo pipefail

[ "$(id -u)" = 0 ] || { echo "run this as root"; exit 1; }

MODEL_REPO="mradermacher/gemma-4-26B-A4B-it-ultra-uncensored-heretic-i1-GGUF"
MODEL_FILE="gemma-4-26B-A4B-it-ultra-uncensored-heretic.i1-IQ4_XS.gguf"
MODEL_URL="https://huggingface.co/${MODEL_REPO}/resolve/main/${MODEL_FILE}"
MODEL_BYTES=13917722144
MODEL_DIR=/root/models
MODEL_PATH="${MODEL_DIR}/${MODEL_FILE}"
LLAMA_DIR=/root/llama.cpp
CTX="${GEMMA_CTX:-32768}"
PORT="${GEMMA_PORT:-8080}"
BIND="${GEMMA_BIND:-0.0.0.0}"
THREADS="${GEMMA_THREADS:-$(nproc)}"

echo "### gemma-uncensored installer  $(date -u +%FT%TZ)"
echo "    ctx=${CTX}  port=${PORT}  bind=${BIND}  threads=${THREADS}"

echo "### [1/7] swap (OOM insurance: the model maps ~14 GB on a 16 GB box)"
if ! swapon --show | grep -q /swapfile; then
  fallocate -l 16G /swapfile
  chmod 600 /swapfile
  mkswap /swapfile >/dev/null
  swapon /swapfile
  grep -q '/swapfile' /etc/fstab || echo '/swapfile none swap sw 0 0' >> /etc/fstab
fi
free -h

echo "### [2/7] packages"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq build-essential cmake git curl libcurl4-openssl-dev ccache >/dev/null
echo "    cpu: $(nproc) cores; $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2 | xargs)"

echo "### [3/7] build llama.cpp (CPU, native arch; ~5-10 min)"
if [ ! -x "${LLAMA_DIR}/build/bin/llama-server" ]; then
  rm -rf "${LLAMA_DIR}"
  git clone --depth 1 https://github.com/ggml-org/llama.cpp "${LLAMA_DIR}"
  cmake -S "${LLAMA_DIR}" -B "${LLAMA_DIR}/build" \
    -DCMAKE_BUILD_TYPE=Release \
    -DGGML_NATIVE=ON \
    -DLLAMA_CURL=ON >/dev/null
  cmake --build "${LLAMA_DIR}/build" --config Release -j "$(nproc)" --target llama-server >/dev/null
fi
"${LLAMA_DIR}/build/bin/llama-server" --version 2>&1 | head -2 || true

echo "### [4/7] download model (${MODEL_FILE}, 13.9 GB; ~5-10 min)"
mkdir -p "${MODEL_DIR}"
if [ ! -f "${MODEL_PATH}" ] || [ "$(stat -c%s "${MODEL_PATH}" 2>/dev/null || echo 0)" != "${MODEL_BYTES}" ]; then
  curl -L --fail --retry 5 --retry-delay 5 -C - -o "${MODEL_PATH}" "${MODEL_URL}"
fi
echo "    size: $(stat -c%s "${MODEL_PATH}") bytes (expected ${MODEL_BYTES})"
[ "$(stat -c%s "${MODEL_PATH}")" = "${MODEL_BYTES}" ] || { echo "download size mismatch; re-run to resume"; exit 1; }

echo "### [5/7] API keys"
if [ ! -s /root/keys.txt ]; then
  { openssl rand -hex 32; openssl rand -hex 32; } > /root/keys.txt
  chmod 600 /root/keys.txt
fi

echo "### [6/7] systemd service"
cat > /etc/systemd/system/gemma.service <<UNIT
[Unit]
Description=gemma-uncensored (llama-server, CPU)
After=network-online.target
Wants=network-online.target
StartLimitIntervalSec=0

[Service]
Type=simple
ExecStart=${LLAMA_DIR}/build/bin/llama-server \\
  --model ${MODEL_PATH} \\
  --alias gemma \\
  --host ${BIND} --port ${PORT} \\
  --ctx-size ${CTX} \\
  --parallel 1 \\
  --threads ${THREADS} \\
  --jinja \\
  --api-key-file /root/keys.txt
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
UNIT
systemctl daemon-reload
systemctl enable gemma >/dev/null 2>&1
systemctl restart gemma

echo "### [7/7] wait for health + smoke test"
KEY1=$(sed -n 1p /root/keys.txt)
KEY2=$(sed -n 2p /root/keys.txt)
for _ in $(seq 1 90); do
  curl -fsS "http://127.0.0.1:${PORT}/health" >/dev/null 2>&1 && break
  sleep 2
done
curl -fsS "http://127.0.0.1:${PORT}/health" && echo
echo "-- chat with key 1 --"
curl -fsS "http://127.0.0.1:${PORT}/v1/chat/completions" \
  -H "Authorization: Bearer ${KEY1}" -H "Content-Type: application/json" \
  -d '{"model":"gemma","messages":[{"role":"user","content":"Reply with exactly: ready"}],"max_tokens":64}' \
  | head -c 300; echo
echo "-- no key (expect 401) --"
curl -s -o /dev/null -w "   http_status=%{http_code}\n" "http://127.0.0.1:${PORT}/v1/models"

PUBLIC_IP=$(curl -fsS -m 3 http://169.254.169.254/metadata/v1/interfaces/public/0/ipv4/address 2>/dev/null \
            || hostname -I | awk '{print $1}')
if [ "${BIND}" = "127.0.0.1" ]; then
  BASE_URL="http://127.0.0.1:${PORT}"
  TUNNEL_NOTE="Private bind. From your laptop first open a tunnel:  ssh -N -L ${PORT}:127.0.0.1:${PORT} root@${PUBLIC_IP}"
else
  BASE_URL="http://${PUBLIC_IP}:${PORT}"
  TUNNEL_NOTE="Public bind: plain HTTP on the droplet's IP, every request needs a key. To restrict who can even reach port ${PORT}, add a DigitalOcean Cloud Firewall allowing only your IP."
fi

cat > /root/GEMMA-READY.txt <<EOF
gemma-uncensored is running.

Base URL : ${BASE_URL}        (OpenAI-compatible: /v1/chat/completions, /v1/models)
Model    : gemma              (any model name works; the server serves its one loaded model)
API key 1: ${KEY1}
API key 2: ${KEY2}

Either key works everywhere. Two exist so you can give one to a second tool and revoke it
on its own later: edit /root/keys.txt (one key per line), then: systemctl restart gemma

${TUNNEL_NOTE}

Chat from your laptop (Rust toolchain from https://rustup.rs; the binary lands in ~/.cargo/bin):
  cargo install --git https://github.com/bobvasic/gemma-uncensored
  gemma --setup          # endpoint: ${BASE_URL}    key: one of the above
  gemma

Operate  : systemctl status gemma · journalctl -u gemma -f · systemctl restart gemma
Billing  : the droplet bills while it exists (idle or powered off too). Delete it to stop.
EOF
chmod 600 /root/GEMMA-READY.txt
echo "### DONE  $(date -u +%FT%TZ) — details in /root/GEMMA-READY.txt"
cat /root/GEMMA-READY.txt
free -h
