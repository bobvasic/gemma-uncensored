//! gemma — fast terminal chat with your self-hosted Gemma 4 Ultra Uncensored Heretic.
//!
//! Part of gemma-uncensored: https://github.com/bobvasic/gemma-uncensored
//!
//! The first run asks for your endpoint and API key and saves them to
//! `~/.config/gemma/config` (mode 600). The environment variables `GEMMA_URL` and
//! `GEMMA_KEY` override the file, which is handy for scripts and CI.
//!
//! ```text
//! gemma                    interactive chat
//! gemma "a question"       one-shot answer on stdout, then exit (reasoning hidden)
//! gemma --setup            enter or change the endpoint and API key
//! gemma --think|--no-think show or hide the model's reasoning scratchpad
//! ```
//!
//! In the chat: `/reset` new conversation · `/think` show/hide reasoning · `/exit`
//! Ctrl-C stops the current reply without quitting.

#![forbid(unsafe_code)]

use clap::Parser;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use serde::Serialize;
use serde_json::{json, Value};
use std::fs;
use std::io::{self, BufRead, BufReader, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// The server serves its one loaded model whatever name is sent.
const MODEL: &str = "gemma";
/// She reasons before answering; leave room for the scratchpad and the answer.
const MAX_TOKENS: u32 = 2048;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(600);

/// "GEMMA" in the ANSI Shadow block font, painted blood-black -> crimson -> red.
const ART: [&str; 6] = [
    " ██████╗ ███████╗███╗   ███╗███╗   ███╗ █████╗ ",
    "██╔════╝ ██╔════╝████╗ ████║████╗ ████║██╔══██╗",
    "██║  ███╗█████╗  ██╔████╔██║██╔████╔██║███████║",
    "██║   ██║██╔══╝  ██║╚██╔╝██║██║╚██╔╝██║██╔══██║",
    "╚██████╔╝███████╗██║ ╚═╝ ██║██║ ╚═╝ ██║██║  ██║",
    " ╚═════╝ ╚══════╝╚═╝     ╚═╝╚═╝     ╚═╝╚═╝  ╚═╝",
];
const GRAD_STOPS: [(u8, u8, u8); 3] = [(74, 0, 12), (176, 12, 32), (255, 58, 48)];

#[derive(Parser)]
#[command(
    name = "gemma",
    version,
    about = "Fast terminal chat with your self-hosted Gemma 4 Ultra Uncensored Heretic.",
    after_help = "Config: ~/.config/gemma/config (or GEMMA_URL / GEMMA_KEY in the environment).\n\
                  In the chat: /reset new conversation · /think show/hide reasoning · /exit\n\
                  Ctrl-C stops the current reply without quitting."
)]
struct Cli {
    /// One-shot prompt; omit it for interactive chat
    prompt: Vec<String>,
    /// Enter or change the endpoint and API key
    #[arg(long)]
    setup: bool,
    /// Show the reasoning scratchpad (one-shot mode hides it by default)
    #[arg(long, conflicts_with = "no_think")]
    think: bool,
    /// Hide the reasoning scratchpad (interactive mode shows it by default)
    #[arg(long)]
    no_think: bool,
}

/// ANSI colors, switched off when stdout is not a terminal so `gemma "q" > out.txt` is clean.
struct Palette {
    dim: &'static str,
    bold: &'static str,
    crimson: &'static str,
    blood: &'static str,
    reset: &'static str,
    truecolor: bool,
}

impl Palette {
    fn detect() -> Self {
        let tty = io::stdout().is_terminal();
        let truecolor = tty
            && std::env::var("COLORTERM")
                .map(|v| {
                    let v = v.to_lowercase();
                    v == "truecolor" || v == "24bit"
                })
                .unwrap_or(false);
        if tty {
            Self {
                dim: "\x1b[90m",
                bold: "\x1b[1m",
                crimson: "\x1b[38;2;206;38;54m",
                blood: "\x1b[38;2;150;22;34m",
                reset: "\x1b[0m",
                truecolor,
            }
        } else {
            Self { dim: "", bold: "", crimson: "", blood: "", reset: "", truecolor: false }
        }
    }
}

fn blend(t: f64) -> String {
    let stops = GRAD_STOPS;
    let last = stops.len() - 1;
    let (r, g, b) = if t <= 0.0 {
        stops[0]
    } else if t >= 1.0 {
        stops[last]
    } else {
        let seg = t * last as f64;
        let i = seg.floor() as usize;
        let f = seg - i as f64;
        let (c0, c1) = (stops[i], stops[i + 1]);
        let mix = |a: u8, b: u8| (f64::from(a) + (f64::from(b) - f64::from(a)) * f).round() as u8;
        (mix(c0.0, c1.0), mix(c0.1, c1.1), mix(c0.2, c1.2))
    };
    format!("\x1b[38;2;{r};{g};{b}m")
}

fn banner(p: &Palette, host: &str) {
    let width = ART.iter().map(|l| l.chars().count()).max().unwrap_or(1).max(2);
    println!();
    for line in ART {
        if p.truecolor {
            let mut out = String::new();
            for (col, ch) in line.chars().enumerate() {
                if ch == ' ' {
                    out.push(' ');
                } else {
                    out.push_str(&blend(col as f64 / (width - 1) as f64));
                    out.push(ch);
                }
            }
            println!("{out}{}", p.reset);
        } else {
            println!("{}{}{line}{}", p.bold, p.crimson, p.reset);
        }
    }
    for text in ["Ultra Uncensored · Heretic".to_string(), format!("self-hosted · {host}")] {
        let pad = width.saturating_sub(text.chars().count()) / 2;
        println!("{}{}{text}{}", " ".repeat(pad), p.blood, p.reset);
    }
    println!();
}

// ---------------------------------------------------------------------------- config

fn config_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".config").join("gemma").join("config"))
}

fn normalize_url(url: &str) -> String {
    let mut u = url.trim().trim_end_matches('/').to_string();
    if let Some(s) = u.strip_suffix("/v1") {
        u = s.to_string();
    }
    if !u.is_empty() && !u.starts_with("http://") && !u.starts_with("https://") {
        u = format!("http://{u}");
    }
    u
}

fn load_config() -> (String, String) {
    let mut url = String::new();
    let mut key = String::new();
    if let Some(text) = config_path().and_then(|p| fs::read_to_string(p).ok()) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                match k.trim() {
                    "url" => url = v.trim().to_string(),
                    "key" => key = v.trim().to_string(),
                    _ => {}
                }
            }
        }
    }
    if let Ok(v) = std::env::var("GEMMA_URL") {
        if !v.is_empty() {
            url = v;
        }
    }
    if let Ok(v) = std::env::var("GEMMA_KEY") {
        if !v.is_empty() {
            key = v;
        }
    }
    (normalize_url(&url), key)
}

fn save_config(url: &str, key: &str) -> io::Result<PathBuf> {
    let path = config_path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "cannot find your home directory"))?;
    let dir = path.parent().expect("config path has a parent");
    let contents =
        format!("# gemma CLI config. Holds a live API key: keep this file private.\nurl={url}\nkey={key}\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
        fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        f.write_all(contents.as_bytes())?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir)?;
        fs::write(&path, contents)?;
    }
    Ok(path)
}

/// Hit /v1/models with the key. Returns (ok, human-readable message).
fn check(agent: &ureq::Agent, url: &str, key: &str) -> (bool, String) {
    match agent
        .get(&format!("{url}/v1/models"))
        .set("Authorization", &format!("Bearer {key}"))
        .call()
    {
        Ok(resp) => (true, format!("connected to {url} (HTTP {})", resp.status())),
        Err(ureq::Error::Status(401, _)) => (false, "the server rejected that key (HTTP 401)".to_string()),
        Err(ureq::Error::Status(code, _)) => (false, format!("HTTP {code} from {url}")),
        Err(ureq::Error::Transport(t)) => (false, format!("cannot reach {url} ({t})")),
    }
}

fn prompt_line(prompt: &str) -> String {
    print!("{prompt}");
    let _ = io::stdout().flush();
    let mut s = String::new();
    let _ = io::stdin().read_line(&mut s);
    s.trim().to_string()
}

fn setup(p: &Palette, agent: &ureq::Agent) -> (String, String) {
    println!("{}gemma setup{}", p.bold, p.reset);
    println!(
        "{}Endpoint is your droplet's IP or hostname on its port, e.g. http://203.0.113.7:8080{}",
        p.dim, p.reset
    );
    println!("{}(from /root/GEMMA-READY.txt on the droplet){}", p.dim, p.reset);
    let url = normalize_url(&prompt_line("Endpoint: "));
    let key = rpassword::prompt_password("API key (hidden): ")
        .unwrap_or_default()
        .trim()
        .to_string();
    let (ok, msg) = check(agent, &url, &key);
    println!("{}{msg}{}", p.dim, p.reset);
    if !ok && prompt_line("Save anyway? [y/N] ").to_lowercase() != "y" {
        println!("Not saved.");
        std::process::exit(1);
    }
    match save_config(&url, &key) {
        Ok(path) => println!("{}saved to {} (mode 600){}\n", p.dim, path.display(), p.reset),
        Err(e) => {
            eprintln!("could not save the config: {e}");
            std::process::exit(1);
        }
    }
    (url, key)
}

// ------------------------------------------------------------------------------ chat

#[derive(Serialize)]
struct Message {
    role: &'static str,
    content: String,
}

/// POST the conversation, stream the reply to stdout, return the answer text.
/// `stop` is raised by Ctrl-C; it is honoured at the next streamed line, which drops the
/// connection so the server cancels the generation too.
fn stream(
    p: &Palette,
    agent: &ureq::Agent,
    url: &str,
    key: &str,
    messages: &[Message],
    show_think: bool,
    stop: &AtomicBool,
) -> String {
    stop.store(false, Ordering::SeqCst);
    let body = json!({
        "model": MODEL,
        "messages": messages,
        "stream": true,
        "max_tokens": MAX_TOKENS,
    });
    let resp = agent
        .post(&format!("{url}/v1/chat/completions"))
        .set("Authorization", &format!("Bearer {key}"))
        .set("Content-Type", "application/json")
        .send_string(&body.to_string());
    let resp = match resp {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let detail: String = r.into_string().unwrap_or_default().chars().take(300).collect();
            let hint = if code == 401 { "  (run: gemma --setup)" } else { "" };
            println!("\n{}[HTTP {code}] {detail}{hint}{}\n", p.dim, p.reset);
            return String::new();
        }
        Err(ureq::Error::Transport(t)) => {
            println!(
                "\n{}[connection error: {t}] is the droplet up? (gemma --setup to change the endpoint){}\n",
                p.dim, p.reset
            );
            return String::new();
        }
    };

    let mut out = io::stdout();
    let mut answer = String::new();
    let mut in_think = false;
    let mut stopped = false;
    let reader = BufReader::new(resp.into_reader());
    for line in reader.lines() {
        if stop.swap(false, Ordering::SeqCst) {
            stopped = true;
            break;
        }
        let Ok(line) = line else { break };
        let Some(data) = line.trim().strip_prefix("data:") else { continue };
        let data = data.trim();
        if data == "[DONE]" {
            break;
        }
        let Ok(v) = serde_json::from_str::<Value>(data) else { continue };
        let delta = &v["choices"][0]["delta"];
        if show_think {
            if let Some(rc) = delta["reasoning_content"].as_str().filter(|s| !s.is_empty()) {
                if !in_think {
                    let _ = write!(out, "{}[thinking] ", p.dim);
                    in_think = true;
                }
                let _ = write!(out, "{rc}");
                let _ = out.flush();
            }
        }
        if let Some(c) = delta["content"].as_str().filter(|s| !s.is_empty()) {
            if in_think {
                let _ = writeln!(out, "{}", p.reset);
                in_think = false;
            }
            let _ = write!(out, "{c}");
            let _ = out.flush();
            answer.push_str(c);
        }
    }
    if in_think {
        let _ = write!(out, "{}", p.reset);
    }
    if stopped {
        println!("{}\n{}[stopped]{}", p.reset, p.dim, p.reset);
    }
    println!();
    answer
}

/// Line editing with history when stdin is a terminal; plain reads when it is piped.
enum Input {
    Editor(DefaultEditor),
    Plain,
}

fn read_input(input: &mut Input, prompt: &str) -> Option<String> {
    match input {
        Input::Editor(rl) => match rl.readline(prompt) {
            Ok(line) => {
                if !line.trim().is_empty() {
                    let _ = rl.add_history_entry(line.as_str());
                }
                Some(line)
            }
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => None,
            Err(e) => {
                eprintln!("{e}");
                None
            }
        },
        Input::Plain => {
            print!("{prompt}");
            let _ = io::stdout().flush();
            let mut s = String::new();
            match io::stdin().read_line(&mut s) {
                Ok(0) | Err(_) => None,
                Ok(_) => Some(s),
            }
        }
    }
}

fn chat(p: &Palette, agent: &ureq::Agent, url: &str, key: &str, mut show_think: bool, stop: &AtomicBool) {
    let host = url.trim_start_matches("http://").trim_start_matches("https://");
    banner(p, host);
    println!(
        "{}/reset new chat · /think toggle reasoning · /exit · Ctrl-C stops a reply{}\n",
        p.dim, p.reset
    );
    let mut input = if io::stdin().is_terminal() {
        DefaultEditor::new().map(Input::Editor).unwrap_or(Input::Plain)
    } else {
        Input::Plain
    };
    let mut messages: Vec<Message> = Vec::new();
    let prompt = format!("{}you ›{} ", p.crimson, p.reset);
    loop {
        let Some(line) = read_input(&mut input, &prompt) else {
            println!("\nbye");
            break;
        };
        let user = line.trim();
        if user.is_empty() {
            continue;
        }
        match user {
            "/exit" | "/quit" => {
                println!("bye");
                break;
            }
            "/reset" => {
                messages.clear();
                println!("{}(conversation cleared){}\n", p.dim, p.reset);
                continue;
            }
            "/think" => {
                show_think = !show_think;
                println!(
                    "{}(reasoning {}){}\n",
                    p.dim,
                    if show_think { "shown" } else { "hidden" },
                    p.reset
                );
                continue;
            }
            _ => {}
        }
        messages.push(Message { role: "user", content: user.to_string() });
        print!("{}{}gemma ›{} ", p.bold, p.crimson, p.reset);
        let _ = io::stdout().flush();
        let reply = stream(p, agent, url, key, &messages, show_think, stop);
        if reply.is_empty() {
            messages.pop(); // nothing came back: drop the turn so history stays clean
        } else {
            messages.push(Message { role: "assistant", content: reply });
        }
    }
}

fn main() {
    let cli = Cli::parse();
    let p = Palette::detect();
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(READ_TIMEOUT)
        .build();

    let (url, key) = if cli.setup {
        setup(&p, &agent)
    } else {
        let (url, key) = load_config();
        if url.is_empty() || key.is_empty() {
            setup(&p, &agent)
        } else {
            (url, key)
        }
    };

    // Installed only now, so Ctrl-C during setup still aborts the program the normal way.
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = Arc::clone(&stop);
        let _ = ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst));
    }

    if cli.prompt.is_empty() {
        chat(&p, &agent, &url, &key, !cli.no_think, &stop);
    } else {
        let show = cli.think && !cli.no_think;
        let messages = vec![Message { role: "user", content: cli.prompt.join(" ") }];
        stream(&p, &agent, &url, &key, &messages, show, &stop);
    }
}
