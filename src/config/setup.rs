//! Interactive onboarding flow for `whisrs setup`.
//!
//! Guides the user through selecting a backend, entering an API key,
//! choosing a language, testing the microphone, and writing `config.toml`.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use dialoguer::{Input, Password, Select};

use crate::{AudioConfig, Config, GeneralConfig, GroqConfig, LocalWhisperConfig, OpenAiConfig};

// ANSI color codes.
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

/// Backend choices presented to the user.
const BACKEND_CHOICES: &[&str] = &[
    "Groq            (free, fast, cloud)",
    "OpenAI Realtime (best streaming, cloud)",
    "OpenAI REST     (simple, cloud)",
    "Local           (offline, no API key needed)",
];

/// Map selection index to backend string used in config.
const BACKEND_VALUES: &[&str] = &["groq", "openai-realtime", "openai", "local"];

/// Whisper model choices (name, file size, description).
const WHISPER_MODEL_CHOICES: &[&str] = &[
    "tiny.en    (75 MB,  decent accuracy, very fast)",
    "base.en    (142 MB, good accuracy, real-time)  <- recommended",
    "small.en   (466 MB, very good accuracy, slower)",
];
const WHISPER_MODEL_NAMES: &[&str] = &["tiny.en", "base.en", "small.en"];

/// Run the full interactive setup flow.
///
/// This function does NOT require the daemon to be running.
pub fn run_setup() -> Result<()> {
    println!("\n{BOLD}whisrs setup{RESET} — interactive onboarding\n");

    // 1. Select backend.
    let backend = select_backend()?;

    // 2. Configure backend (API key or model download).
    let (groq_config, openai_config, local_whisper_config) = configure_backend(&backend)?;

    // 3. Language.
    let language = select_language()?;

    // 4. Test microphone.
    test_microphone();

    // 5. Build and write config.
    let config = Config {
        general: GeneralConfig {
            backend,
            language,
            silence_timeout_ms: 2000,
            notify: true,
        },
        audio: AudioConfig {
            device: "default".to_string(),
        },
        groq: groq_config,
        openai: openai_config,
        local_whisper: local_whisper_config,
        local_vosk: None,
        local_parakeet: None,
    };

    let config_path = write_config(&config)?;
    println!(
        "\n{GREEN}Config written to {}{RESET}",
        config_path.display()
    );

    // 6. Check uinput permissions.
    check_uinput_permissions();

    // 7. Print next steps.
    print_next_steps();

    Ok(())
}

/// Prompt the user to select a transcription backend.
fn select_backend() -> Result<String> {
    let selection = Select::new()
        .with_prompt("Select a transcription backend")
        .items(BACKEND_CHOICES)
        .default(0)
        .interact()
        .context("failed to read backend selection")?;

    let mut backend = BACKEND_VALUES[selection].to_string();

    // If "local" selected, show engine sub-menu.
    if backend == "local" {
        backend = select_local_engine()?;
    }

    println!("  {DIM}Selected: {backend}{RESET}");
    Ok(backend)
}

/// Sub-menu for choosing a local transcription engine.
fn select_local_engine() -> Result<String> {
    println!();
    let selection = Select::new()
        .with_prompt("Select a local engine")
        .items(&[
            "whisper.cpp     (recommended — best accuracy, CPU/GPU)",
            "Vosk            (coming soon — true streaming, tiny model)",
            "Parakeet        (coming soon — NVIDIA, ultra-fast)",
        ])
        .default(0)
        .interact()
        .context("failed to read engine selection")?;

    match selection {
        0 => {
            // Check if the binary was compiled with local-whisper support.
            if !cfg!(feature = "local-whisper") {
                println!();
                println!("  {RED}This binary was compiled without local whisper support.{RESET}");
                println!();
                println!("  Rebuild with the feature flag:");
                println!("    {BOLD}cargo install --path . --features local-whisper{RESET}");
                println!();
                println!("  Build requirements: libclang, cmake, C++ compiler");
                println!("    Arch:   {DIM}sudo pacman -S clang cmake{RESET}");
                println!(
                    "    Debian: {DIM}sudo apt install libclang-dev cmake build-essential{RESET}"
                );
                println!("    Fedora: {DIM}sudo dnf install clang-devel cmake gcc-c++{RESET}");
                println!();
                anyhow::bail!(
                    "rerun `whisrs setup` after rebuilding with --features local-whisper"
                );
            }
            Ok("local-whisper".to_string())
        }
        1 => {
            println!(
                "  {YELLOW}Vosk support is coming in a future release. Selecting whisper.cpp instead.{RESET}"
            );
            Ok("local-whisper".to_string())
        }
        _ => {
            println!(
                "  {YELLOW}Parakeet support is coming in a future release. Selecting whisper.cpp instead.{RESET}"
            );
            Ok("local-whisper".to_string())
        }
    }
}

/// Configure the selected backend (API key or model path).
fn configure_backend(
    backend: &str,
) -> Result<(
    Option<GroqConfig>,
    Option<OpenAiConfig>,
    Option<LocalWhisperConfig>,
)> {
    match backend {
        "groq" => {
            let api_key = prompt_api_key(
                "Groq API key",
                "Get one free at https://console.groq.com/keys",
            )?;
            Ok((
                Some(GroqConfig {
                    api_key,
                    model: "whisper-large-v3-turbo".to_string(),
                }),
                None,
                None,
            ))
        }
        "openai-realtime" | "openai" => {
            let api_key = prompt_api_key(
                "OpenAI API key",
                "Get one at https://platform.openai.com/api-keys",
            )?;
            let model = if backend == "openai-realtime" {
                "gpt-4o-mini-transcribe".to_string()
            } else {
                let selection = Select::new()
                    .with_prompt("Select OpenAI model")
                    .items(&[
                        "gpt-4o-mini-transcribe (recommended)",
                        "gpt-4o-transcribe",
                        "whisper-1",
                    ])
                    .default(0)
                    .interact()
                    .context("failed to read model selection")?;
                match selection {
                    0 => "gpt-4o-mini-transcribe",
                    1 => "gpt-4o-transcribe",
                    _ => "whisper-1",
                }
                .to_string()
            };
            Ok((None, Some(OpenAiConfig { api_key, model }), None))
        }
        "local-whisper" => {
            // Select model size.
            println!();
            let model_idx = Select::new()
                .with_prompt("Select a whisper model")
                .items(WHISPER_MODEL_CHOICES)
                .default(1) // base.en is recommended
                .interact()
                .context("failed to read model selection")?;

            let model_name = WHISPER_MODEL_NAMES[model_idx];

            let model_dir = default_model_dir();
            let dest = model_dir.join(format!("ggml-{model_name}.bin"));

            if dest.exists() {
                println!("  {GREEN}Model already exists at {}{RESET}", dest.display());
            } else {
                // Offer to download.
                let should_download = Select::new()
                    .with_prompt("Download model now?")
                    .items(&["Yes, download now", "No, I'll download it manually"])
                    .default(0)
                    .interact()
                    .context("failed to read download choice")?;

                if should_download == 0 {
                    download_whisper_model(model_name, &model_dir)?;
                } else {
                    println!("  {DIM}Download the model manually from:{RESET}");
                    println!(
                        "  {DIM}https://huggingface.co/ggerganov/whisper.cpp/tree/main{RESET}"
                    );
                    println!("  {DIM}Place it at: {}{RESET}", dest.display());
                }
            }

            let model_path = dest.to_string_lossy().to_string();
            Ok((None, None, Some(LocalWhisperConfig { model_path })))
        }
        _ => Ok((None, None, None)),
    }
}

/// Return the default directory for storing whisper models.
fn default_model_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("~/.local/share"))
        .join("whisrs/models")
}

/// Download a whisper.cpp GGML model from HuggingFace.
fn download_whisper_model(model_name: &str, model_dir: &std::path::Path) -> Result<()> {
    use std::io::{Read, Write};

    let url =
        format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-{model_name}.bin");
    let dest = model_dir.join(format!("ggml-{model_name}.bin"));

    fs::create_dir_all(model_dir)
        .with_context(|| format!("failed to create model directory {}", model_dir.display()))?;

    println!("\n  Downloading ggml-{model_name}.bin from HuggingFace...");

    // Run download in a separate thread to avoid conflict with tokio runtime.
    let dest_clone = dest.clone();
    let url_clone = url.clone();
    std::thread::spawn(move || -> Result<()> {
        let response = reqwest::blocking::Client::builder()
            .user_agent("whisrs")
            .build()
            .context("failed to build HTTP client")?
            .get(&url_clone)
            .send()
            .context("failed to connect to HuggingFace — check your internet connection")?;

        if !response.status().is_success() {
            anyhow::bail!(
                "download failed: HTTP {} from {url_clone}",
                response.status()
            );
        }

        let total_size = response.content_length().unwrap_or(0);

        let pb = indicatif::ProgressBar::new(total_size);
        pb.set_style(
            indicatif::ProgressStyle::with_template(
                "  [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})",
            )
            .unwrap()
            .progress_chars("=> "),
        );

        let mut file = fs::File::create(&dest_clone)
            .with_context(|| format!("failed to create {}", dest_clone.display()))?;

        let mut reader = std::io::BufReader::new(response);
        let mut buf = [0u8; 8192];

        loop {
            let n = reader.read(&mut buf).context("download interrupted")?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])
                .context("failed to write model file")?;
            pb.inc(n as u64);
        }

        pb.finish_and_clear();
        Ok(())
    })
    .join()
    .map_err(|_| anyhow::anyhow!("download thread panicked"))??;

    println!("  {GREEN}Model saved to {}{RESET}", dest.display());
    println!("  {DIM}No API key needed — everything runs on your machine.{RESET}");

    Ok(())
}

/// Prompt for an API key using hidden input.
fn prompt_api_key(prompt: &str, hint: &str) -> Result<String> {
    println!("  {DIM}{hint}{RESET}");
    let key = Password::new()
        .with_prompt(prompt)
        .interact()
        .context("failed to read API key")?;
    if key.is_empty() {
        println!("  {YELLOW}Warning: empty API key — you can set it later in config.toml{RESET}");
    }
    Ok(key)
}

/// Ask the user for their preferred language.
fn select_language() -> Result<String> {
    let language: String = Input::new()
        .with_prompt("Language (ISO 639-1 code, or \"auto\" for auto-detect)")
        .default("en".to_string())
        .interact_text()
        .context("failed to read language")?;
    Ok(language)
}

/// Attempt to open the default audio input device and report success/failure.
fn test_microphone() {
    use cpal::traits::{DeviceTrait, HostTrait};

    println!("\n{BOLD}Testing microphone...{RESET}");

    let host = cpal::default_host();
    match host.default_input_device() {
        Some(device) => {
            let name = device.name().unwrap_or_else(|_| "unknown".into());
            println!("  {GREEN}Microphone OK:{RESET} {name}");

            // Try to get a supported config to verify the device actually works.
            match device.default_input_config() {
                Ok(config) => {
                    println!(
                        "  {DIM}Format: {} Hz, {} channel(s){RESET}",
                        config.sample_rate().0,
                        config.channels()
                    );
                }
                Err(e) => {
                    println!("  {YELLOW}Warning: could not query device config: {e}{RESET}");
                }
            }
        }
        None => {
            println!("  {RED}No default audio input device found.{RESET}");

            // List available devices.
            if let Ok(devices) = host.input_devices() {
                let names: Vec<String> = devices.filter_map(|d| d.name().ok()).collect();
                if names.is_empty() {
                    println!(
                        "  No input devices detected. Check that your microphone is connected"
                    );
                    println!("  and that PipeWire/PulseAudio is running.");
                } else {
                    println!("  Available input devices:");
                    for name in &names {
                        println!("    - {name}");
                    }
                    println!(
                        "  {DIM}Set the device in config.toml under [audio] device = \"...\"{RESET}"
                    );
                }
            }
        }
    }
}

/// Write the config to `~/.config/whisrs/config.toml` with `chmod 0600`.
fn write_config(config: &Config) -> Result<PathBuf> {
    let config_path = crate::config_path();
    let config_dir = config_path
        .parent()
        .expect("config path should have a parent directory");

    // Create the config directory if it doesn't exist.
    fs::create_dir_all(config_dir)
        .with_context(|| format!("failed to create config directory {}", config_dir.display()))?;

    // Serialize config to TOML.
    let toml_str = toml::to_string_pretty(config).context("failed to serialize config to TOML")?;

    // Write the file.
    fs::write(&config_path, &toml_str)
        .with_context(|| format!("failed to write config to {}", config_path.display()))?;

    // Set permissions to 0600 (owner read/write only) since it may contain API keys.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&config_path, perms)
            .with_context(|| format!("failed to set permissions on {}", config_path.display()))?;
    }

    Ok(config_path)
}

/// Check if /dev/uinput is accessible and print guidance if not.
fn check_uinput_permissions() {
    use std::fs::OpenOptions;

    println!("\n{BOLD}Checking uinput permissions...{RESET}");

    match OpenOptions::new().write(true).open("/dev/uinput") {
        Ok(_) => {
            println!("  {GREEN}uinput access: OK{RESET}");
        }
        Err(e) => {
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                println!("  {RED}Cannot open /dev/uinput — permission denied.{RESET}");
                println!();
                println!("  Fix with one of:");
                println!();
                println!("  1. Add yourself to the input group:");
                println!("     sudo usermod -aG input $USER");
                println!("     # Then log out and log back in");
                println!();
                println!("  2. Install the udev rule (included in contrib/):");
                println!("     sudo cp contrib/99-whisrs.rules /etc/udev/rules.d/");
                println!("     sudo udevadm control --reload-rules");
                println!("     sudo udevadm trigger");
            } else {
                println!("  {YELLOW}Cannot open /dev/uinput: {e}{RESET}");
            }
        }
    }
}

/// Print the final "you're ready" message with next steps.
fn print_next_steps() {
    println!("\n{GREEN}{BOLD}You're ready!{RESET}");
    println!();
    println!("  Start the daemon:");
    println!("    whisrsd &");
    println!("  Or enable the systemd service:");
    println!("    systemctl --user enable --now whisrs.service");
    println!();
    println!("  Then bind {BOLD}whisrs toggle{RESET} to a hotkey in your WM/DE.");
    println!();
    println!("  {DIM}Config: ~/.config/whisrs/config.toml{RESET}");
    println!("  {DIM}Logs:   RUST_LOG=debug whisrsd{RESET}");
    println!();
}
