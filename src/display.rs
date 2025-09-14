use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

#[derive(Debug)]
pub enum ConfigurationStatus {
    Started(String),
    Evaluating,
    Evaluated { drvPath: String },
    Building,
    Built { result: String },
    Activating,
    Activated,
    Error { error: String },
}

pub struct StatusHandle {
    pub config_name: String,
    pub tx: tokio::sync::mpsc::UnboundedSender<ConfigurationStatus>,
}

pub struct StatusManager {
    multi_progress: MultiProgress,
    spinner_style: ProgressStyle,
}

impl StatusManager {
    pub fn new() -> Self {
        let spinner_style =
            ProgressStyle::with_template("[{elapsed}] {prefix:.bold.dim} {spinner} {wide_msg}")
                .unwrap()
                .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ");
        let multi_progress = MultiProgress::new();

        Self {
            multi_progress,
            spinner_style,
        }
    }

    pub fn add_configuration(&self, name: &str) -> StatusHandle {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let pb = self.multi_progress.add(ProgressBar::new(100));
        pb.set_style(self.spinner_style.clone());
        pb.enable_steady_tick(Duration::from_millis(100));
        //pb.set_prefix(format!("[{}/?]", i + 1));

        tokio::spawn(async move {
            while let Some(status) = rx.recv().await {
                // Update the progress bar based on the status
                match status {
                    ConfigurationStatus::Started(name) => {
                        // Initialize progress bar
                        pb.set_prefix(format!("[{}]", name));
                    }
                    ConfigurationStatus::Evaluating => {
                        // Update to evaluating state
                        pb.set_message("Evaluating...");
                    }
                    ConfigurationStatus::Evaluated { drvPath: _ } => {
                        // Update to evaluated state
                        pb.set_message("Evaluated, starting build...");
                    }
                    ConfigurationStatus::Building => {
                        // Update to building state
                        pb.set_message("Building...");
                    }
                    ConfigurationStatus::Built { result: _ } => {
                        // Update to built state
                        pb.set_message("Build complete, activating...");
                    }
                    ConfigurationStatus::Activating => {
                        // Update to activating state
                        pb.set_message("Activating...");
                    }
                    ConfigurationStatus::Activated => {
                        // Finalize progress bar
                        pb.finish_with_message("Activated");
                    }
                    ConfigurationStatus::Error { error } => {
                        // Handle error state
                        pb.abandon_with_message(format!("Error occurred: {error}"));
                    }
                }
            }
            //pb.abandon_with_message("Done");
        });

        StatusHandle {
            config_name: name.to_string(),
            tx,
        }
    }

    pub fn shutdown(self) {
        self.multi_progress.clear().unwrap();
    }
}
