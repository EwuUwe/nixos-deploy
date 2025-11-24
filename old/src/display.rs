use std::sync::{Arc, Mutex};
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;
use tokio::sync::mpsc;
use tracing::warn;

use crate::types::{ConfigurationStatus, Verbosity};

#[derive(Clone)]
pub struct ErrorInfo {
    pub config_name: String,
    pub error: String,
    pub error_lines: Vec<String>,
}

pub struct StatusHandle {
    pub config_name: String,
    pub tx: mpsc::UnboundedSender<ConfigurationStatus>,
}

pub struct StatusManager {
    multi_progress: MultiProgress,
    spinner_style: ProgressStyle,
    verbosity: Verbosity,
    buffered_errors: Arc<Mutex<Vec<ErrorInfo>>>,
}

impl StatusManager {
    pub fn new(verbosity: Verbosity) -> Self {
        let spinner_style = match verbosity {
            Verbosity::Progress => {
                ProgressStyle::with_template("[{elapsed}] {prefix:.bold.dim} {spinner} {wide_msg}")
                    .unwrap()
                    .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ")
            }
            Verbosity::Detailed => {
                ProgressStyle::with_template("[{elapsed}] {prefix:.bold.dim} {wide_msg}").unwrap()
            }
        };

        let multi_progress = MultiProgress::new();

        Self {
            multi_progress,
            spinner_style,
            verbosity,
            buffered_errors: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn add_configuration(&self, name: &str) -> StatusHandle {
        let (tx, mut rx) = mpsc::unbounded_channel();

        match self.verbosity {
            Verbosity::Progress => {
                let pb = self.multi_progress.add(ProgressBar::new(100));
                pb.set_style(self.spinner_style.clone());
                pb.enable_steady_tick(Duration::from_millis(100));

                let pb_clone = pb.clone();
                let buffered_errors = self.buffered_errors.clone();
                let config_name = name.to_string();
                tokio::spawn(async move {
                    let mut last_status: Option<String> = None;
                    while let Some(status) = rx.recv().await {
                        // Buffer errors instead of printing them immediately
                        if let ConfigurationStatus::Error { error, error_lines } = &status {
                            let error_info = ErrorInfo {
                                config_name: config_name.clone(),
                                error: error.clone(),
                                error_lines: error_lines.clone(),
                            };
                            if let Ok(mut errors) = buffered_errors.lock() {
                                errors.push(error_info);
                            }
                        }

                        // Only update progress bar if status actually changed
                        let status_str = format!("{:?}", status);
                        if last_status.as_ref() != Some(&status_str) {
                            Self::update_progress_bar(&pb_clone, &status);
                            last_status = Some(status_str);
                        }
                    }
                });

                // IMPORTANT: Do NOT spawn detailed logging task in Progress mode
            }
            Verbosity::Detailed => {
                let config_name = name.to_string();
                tokio::spawn(async move {
                    while let Some(status) = rx.recv().await {
                        Self::log_detailed_status(&status, &config_name);
                    }
                });

                // IMPORTANT: Do NOT spawn progress bar task in Detailed mode
            }
        }

        StatusHandle {
            config_name: name.to_string(),
            tx,
        }
    }

    fn update_progress_bar(pb: &ProgressBar, status: &ConfigurationStatus) {
        match status {
            ConfigurationStatus::Started(name) => {
                pb.set_prefix(format!("[{}]", name));
                pb.set_message("Starting...");
            }
            ConfigurationStatus::Evaluating => {
                pb.set_message("Evaluating configuration...");
            }
            ConfigurationStatus::Evaluated { drv_path: _ } => {
                pb.set_message("Configuration evaluated");
            }
            ConfigurationStatus::Building => {
                pb.set_message("Building derivation...");
            }
            ConfigurationStatus::Built { result: _ } => {
                pb.set_message("Derivation built");
            }
            ConfigurationStatus::CopyingClosure => {
                pb.set_message("Copying closure to target...");
            }
            ConfigurationStatus::Activating => {
                pb.set_message("Activating configuration...");
            }
            ConfigurationStatus::Activated => {
                pb.finish_with_message("✓ Deployed successfully");
            }
            ConfigurationStatus::Error {
                error: _,
                error_lines: _,
            } => {
                pb.abandon_with_message("✗ Failed");
            }
        }
    }

    fn log_detailed_status(status: &ConfigurationStatus, config_name: &str) {
        match status {
            ConfigurationStatus::Started(_) => {
                println!(
                    "{} {}: Starting deployment",
                    format!("[{}]", config_name).bright_black(),
                    "INFO".bright_blue()
                );
            }
            ConfigurationStatus::Evaluating => {
                println!(
                    "{} {}: → Evaluating configuration...",
                    format!("[{}]", config_name).bright_black(),
                    "INFO".bright_blue()
                );
            }
            ConfigurationStatus::Evaluated { drv_path } => {
                println!(
                    "{} {}: → Configuration evaluated: {}",
                    format!("[{}]", config_name).bright_black(),
                    "INFO".bright_blue(),
                    drv_path
                );
            }
            ConfigurationStatus::Building => {
                println!(
                    "{} {}: → Building configuration...",
                    format!("[{}]", config_name).bright_black(),
                    "INFO".bright_blue()
                );
            }
            ConfigurationStatus::Built { result } => {
                println!(
                    "{} {}: → Configuration built: {}",
                    format!("[{}]", config_name).bright_black(),
                    "INFO".bright_blue(),
                    result
                );
            }
            ConfigurationStatus::CopyingClosure => {
                println!(
                    "{} {}: → Copying closure to target...",
                    format!("[{}]", config_name).bright_black(),
                    "INFO".bright_blue()
                );
            }
            ConfigurationStatus::Activating => {
                println!(
                    "{} {}: → Activating configuration...",
                    format!("[{}]", config_name).bright_black(),
                    "INFO".bright_blue()
                );
            }
            ConfigurationStatus::Activated => {
                println!(
                    "{} {}: ✓ Configuration deployed successfully",
                    format!("[{}]", config_name).bright_black(),
                    "INFO".bright_blue()
                );
            }
            ConfigurationStatus::Error { error, error_lines } => {
                eprintln!(
                    "{} {}: {}",
                    format!("[{}]", config_name).bright_black(),
                    "ERROR".bright_red(),
                    error
                );
                if !error_lines.is_empty() {
                    eprintln!(
                        "{} {}: Error details:",
                        format!("[{}]", config_name).bright_black(),
                        "ERROR".bright_red()
                    );
                    for line in error_lines {
                        eprintln!(
                            "{} {}: │ {}",
                            format!("[{}]", config_name).bright_black(),
                            "ERROR".bright_red(),
                            line
                        );
                    }
                }
            }
        }
    }

    fn truncate_path(path: &str, max_len: usize) -> String {
        if path.len() <= max_len {
            path.to_string()
        } else {
            format!("...{}", &path[path.len() - (max_len - 3)..])
        }
    }

    pub fn shutdown(self) {
        if let Err(e) = self.multi_progress.clear() {
            warn!("Failed to clear progress display: {}", e);
        }

        // In Progress mode, show buffered errors after clearing progress bars
        if matches!(self.verbosity, Verbosity::Progress) {
            if let Ok(errors) = self.buffered_errors.lock() {
                if !errors.is_empty() {
                    println!("\n{}", "Deployment Errors:".bright_red().bold());

                    for error_info in errors.iter() {
                        println!(
                            "\n{} {}:",
                            "●".bright_red(),
                            error_info.config_name.bright_white().bold()
                        );

                        // Extract the most useful error information
                        let useful_error =
                            Self::extract_useful_error(&error_info.error, &error_info.error_lines);
                        for line in useful_error {
                            if !line.trim().is_empty() {
                                println!("  {}", line);
                            }
                        }
                    }
                }
            }
        }
    }

    fn extract_useful_error(main_error: &str, error_lines: &[String]) -> Vec<String> {
        // If we have detailed error lines, prefer those
        if !error_lines.is_empty() {
            return error_lines
                .iter()
                .map(|line| {
                    // Remove redundant host prefix like "[nixos-desktop] Error: " since we're already grouped by host
                    if let Some(colon_pos) = line.find("] ") {
                        if line.starts_with('[') {
                            line[colon_pos + 2..].to_string()
                        } else {
                            line.clone()
                        }
                    } else {
                        line.clone()
                    }
                })
                .collect();
        }

        // Otherwise, try to extract the meaningful part from the main error
        if main_error.contains("nix-build failed:") {
            // Extract just the actual nix error part
            if let Some(start) = main_error.find("nix-build failed:") {
                let nix_error = &main_error[start + "nix-build failed:".len()..].trim();
                return vec![nix_error.to_string()];
            }
        }

        if main_error.contains("Command execution failed:") {
            // Extract just the command that failed
            if let Some(start) = main_error.find("Command execution failed:") {
                let cmd_error = &main_error[start + "Command execution failed:".len()..].trim();
                return vec![format!("Command failed: {}", cmd_error)];
            }
        }

        // Fallback to the main error
        vec![main_error.to_string()]
    }
}
