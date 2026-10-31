# gemma-uncensored

Run **Gemma 4 Ultra Uncensored (Heretic)** on a cheap DigitalOcean CPU droplet — no GPU — as a
private OpenAI-compatible API, and talk to it from your terminal with a zero-dependency chat
client. One paste to deploy, one command to chat.

[![Deploy to DigitalOcean](https://www.deploytodo.com/do-btn-blue.svg)](https://cloud.digitalocean.com/droplets/new?image=ubuntu-24-04-x64&size=s-8vcpu-16gb-amd)

```
 ██████╗ ███████╗███╗   ███╗███╗   ███╗ █████╗
██╔════╝ ██╔════╝████╗ ████║████╗ ████║██╔══██╗
██║  ███╗█████╗  ██╔████╔██║██╔████╔██║███████║
██║   ██║██╔══╝  ██║╚██╔╝██║██║╚██╔╝██║██╔══██║
╚██████╔╝███████╗██║ ╚═╝ ██║██║ ╚═╝ ██║██║  ██║
 ╚═════╝ ╚══════╝╚═╝     ╚═╝╚═╝     ╚═╝╚═╝  ╚═╝
```

## What you get

- **The model:** [`llmfan46/gemma-4-26B-A4B-it-ultra-uncensored-heretic`](https://huggingface.co/llmfan46/gemma-4-26B-A4B-it-ultra-uncensored-heretic)
  — Gemma 4 26B (a mixture-of-experts with ~4B active parameters), decensored with the
  Heretic ARA method. It keeps the base model's capability and refuses almost nothing
  (3/100 on the standard refusal set). Served as the IQ4_XS GGUF (13.9 GB) from
  [mradermacher](https://huggingface.co/mradermacher/gemma-4-26B-A4B-it-ultra-uncensored-heretic-i1-GGUF),
  which is why it fits a 16 GB box.
- **The server:** [llama.cpp](https://github.com/ggml-org/llama.cpp) built for the droplet's
  CPU, run by systemd, exposing `/v1/chat/completions` and `/v1/models` behind API keys that
  the installer generates for you.
- **The client:** `gemma`, a small Rust terminal chat that compiles to one static binary.
  Streams the reply as it is written, shows the model's reasoning scratchpad dimmed, keeps
  the conversation (with line editing and history), and also works one-shot:
  `gemma "your question"`.

## What to expect (be honest with yourself before paying)

| | |
|---|---|
| Cost | `s-8vcpu-16gb-amd` is **$0.167/hour ≈ $4/day ≈ $120/month**, billed while the droplet exists — idle and powered-off too. **Delete it when you're done.** |
| Writing speed | **~9 tokens/second.** Comfortable for chat; a long answer takes a minute. |
| Reading speed | **~35 tokens/second** of prompt. A 500-token question starts answering in a few seconds. A 15,000-token prompt (what coding agents send) takes ~7 minutes before the first word. **This is a chat box, not an agent box.** |
| Context | 32,768 tokens by default. |
| Thinking | The model reasons before it answers. The scratchpad comes back separately (`reasoning_content`), the answer in `content`. |
| Setup time | ~15–20 minutes on first boot: 13.9 GB download + a llama.cpp build. |

Want 40–60 tokens/s and instant prompt reading? The same files run on a GPU droplet
(RTX 4000 Ada, $0.76/h) with a CUDA build — out of scope here, but nothing in the design
prevents it.

## Deploy

[![Deploy to DigitalOcean](https://www.deploytodo.com/do-btn-blue.svg)](https://cloud.digitalocean.com/droplets/new?image=ubuntu-24-04-x64&size=s-8vcpu-16gb-amd)

You need a [DigitalOcean](https://www.digitalocean.com/) account with an SSH key added to it.

### With the button (control panel, one paste)

1. Click the button. It opens **Create Droplet** with **Ubuntu 24.04** and the **8 vCPU / 16 GB**
   size pre-selected; pick the region nearest you and your SSH key.
2. Open **Advanced Options → Add Initialization scripts (free)** and paste
   [`install/cloud-init.yaml`](https://raw.githubusercontent.com/bobvasic/gemma-uncensored/main/install/cloud-init.yaml)
   (that link is the raw file — select all, copy).
3. Click **Create Droplet**. That is everything: the droplet downloads the model, builds the
   server, generates your keys and starts serving on its own (~15–20 min).

### With `doctl` (one command)

```bash
git clone https://github.com/bobvasic/gemma-uncensored.git && cd gemma-uncensored
doctl compute droplet create gemma \
  --region lon1 --size s-8vcpu-16gb-amd --image ubuntu-24-04-x64 \
  --ssh-keys "$(doctl compute ssh-key list --format ID --no-header | head -1)" \
  --user-data-file install/cloud-init.yaml --wait
```

Any region works. `s-8vcpu-16gb-amd` (~$120/mo) is the recommended size: 16 GB is the floor
for this model, and the Premium AMD variant has newer CPUs. The regular `s-8vcpu-16gb`
(~$96/mo) also fits but has an older CPU, so expect fewer tokens per second. Bigger CPU
sizes are faster.

### Then: get your keys

```bash
ssh root@<droplet-ip> tail -f /var/log/gemma-install.log   # watch progress (optional)
ssh root@<droplet-ip> cat /root/GEMMA-READY.txt             # when it says DONE
```

`GEMMA-READY.txt` holds the base URL and two API keys. Either key works; two exist so you
can hand one to a second tool and revoke it on its own later.

## Chat

Install the client with Cargo (Rust toolchain from [rustup.rs](https://rustup.rs); the binary
lands in `~/.cargo/bin`):

```bash
cargo install --git https://github.com/bobvasic/gemma-uncensored
gemma --setup      # paste the base URL and one key from GEMMA-READY.txt
gemma
```

Or from a clone: `cargo build --release` puts the binary at `target/release/gemma`.

The first run asks for the endpoint and key and saves them to `~/.config/gemma/config`
(mode 600). `GEMMA_URL` and `GEMMA_KEY` in the environment override it.

```
gemma                     interactive chat with the banner
gemma "a question"        one-shot answer to stdout (reasoning hidden; add --think to see it)
gemma --no-think          interactive, scratchpad hidden
gemma --setup             change endpoint or key
```

In the chat: `/reset` starts a new conversation, `/think` shows or hides the reasoning,
`/exit` quits, arrow keys edit and recall earlier prompts, and **Ctrl-C** stops the current
reply without quitting (it takes effect at the next streamed token).

## Use it from anything else

It is a plain OpenAI-compatible endpoint. Any model name is accepted — the server serves its
one loaded model whatever you call it.

```bash
curl http://<droplet-ip>:8080/v1/chat/completions \
  -H "Authorization: Bearer <key>" -H "Content-Type: application/json" \
  -d '{"model":"gemma","messages":[{"role":"user","content":"hello"}],"max_tokens":512,"stream":false}'
```

Give it generous `max_tokens`: the reasoning scratchpad counts, and a short cap cuts the
answer off mid-thought.

**Coding agents (OpenCode, Hermes, and friends)** work, but every turn sends them a
10–15k-token system prompt, which this CPU box reads at ~35 tokens/s — expect several
minutes per turn. Point them at it only if you can wait.

## Security

- Requests without a valid key get **401**. Keys are 32 random bytes each, generated on the
  droplet, stored only in `/root/keys.txt` (root, mode 600) and `GEMMA-READY.txt`.
- The default bind is `0.0.0.0`: the API is reachable on the droplet's public IP over
  **plain HTTP**, so keys and prompts cross the internet unencrypted. For a throwaway box
  that is usually acceptable; if it is not:
  - restrict who can reach port 8080 with a **DigitalOcean Cloud Firewall** (allow your IP only), or
  - deploy with `GEMMA_BIND=127.0.0.1` (see the env line in `cloud-init.yaml`) and reach it
    over an SSH tunnel: `ssh -N -L 8080:127.0.0.1:8080 root@<droplet-ip>`, then use
    `http://127.0.0.1:8080` as the endpoint.
- Rotate keys any time: edit `/root/keys.txt`, `systemctl restart gemma`, re-run `gemma --setup`.

## Operate

On the droplet:

| | |
|---|---|
| Status / live log | `systemctl status gemma` · `journalctl -u gemma -f` |
| Restart | `systemctl restart gemma` |
| Change context, port, bind, threads | `GEMMA_CTX=16384 GEMMA_BIND=127.0.0.1 bash /root/setup.sh` (re-runs are safe; build and download are skipped) |
| Re-read the keys | `cat /root/GEMMA-READY.txt` |
| **Stop paying** | `doctl compute droplet delete gemma` (or delete it in the control panel). Nothing else stops the bill. |

## Why this model

Picked for "the strongest uncensored model that fits 16 GB of RAM on a CPU", checked against
the UGI leaderboard (top CPU-sized entry at the time), abliterlitics.dev (Heretic's ARA keeps
Gemma 4's capability better than the alternatives — MMLU 82.5 → 80.0), and r/LocalLLaMA's
pick for 16 GB. Like every decensored model, it states things confidently that can be
wrong; verify anything that matters.

## Layout

```
src/main.rs             the terminal client (Rust); `cargo install` builds it into the `gemma` binary
Cargo.toml              the package (binary name `gemma`)
install/setup.sh        what the droplet runs on first boot; also safe to run by hand as root
install/cloud-init.yaml the one-paste deploy file (fetches setup.sh and runs it, logged)
```

## License

The tool is [MIT](LICENSE). The model weights are **not** part of this repository: the
installer downloads them from Hugging Face at deploy time, and by using them you accept
[Google's Gemma Terms of Use](https://ai.google.dev/gemma/terms) and the terms of the
third-party finetune and quantization linked above. You are responsible for how you use an
uncensored model.

## Credits

[llmfan46](https://huggingface.co/llmfan46) (the Heretic finetune) ·
[mradermacher](https://huggingface.co/mradermacher) (the GGUF quants) ·
[Heretic](https://github.com/p-e-w/heretic) (the decensoring method) ·
[llama.cpp](https://github.com/ggml-org/llama.cpp) (the server).
