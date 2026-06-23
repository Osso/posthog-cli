mod api;
mod config;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tabled::{Table, Tabled};

use api::{FeatureFlag, PostHogClient};
use config::Config;

#[derive(Parser)]
#[command(name = "posthog")]
#[command(about = "Manage PostHog feature flags")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage feature flags
    Flags {
        #[command(subcommand)]
        command: FlagsCommands,
    },
}

#[derive(Subcommand)]
enum FlagsCommands {
    /// List all feature flags
    List,
    /// Get details of a specific flag
    Get {
        /// Flag key
        key: String,
    },
    /// Create a new boolean feature flag (enabled for all by default)
    Create {
        /// Flag key
        key: String,
    },
    /// Enable a feature flag
    Enable {
        /// Flag key
        key: String,
    },
    /// Disable a feature flag
    Disable {
        /// Flag key
        key: String,
    },
    /// Delete a feature flag
    Delete {
        /// Flag key
        key: String,
    },
}

#[derive(Tabled)]
struct FlagRow {
    #[tabled(rename = "Key")]
    key: String,
    #[tabled(rename = "Name")]
    name: String,
    #[tabled(rename = "Active")]
    active: String,
    #[tabled(rename = "Created")]
    created: String,
}

const DATE_PREFIX_LEN: usize = 10; // "YYYY-MM-DD"

fn flag_to_row(f: &FeatureFlag) -> FlagRow {
    let created = if f.created_at.len() >= DATE_PREFIX_LEN {
        f.created_at[..DATE_PREFIX_LEN].to_string()
    } else {
        "-".to_string()
    };
    FlagRow {
        key: f.key.clone(),
        name: f.name.clone(),
        active: if f.active {
            "yes".to_string()
        } else {
            "no".to_string()
        },
        created,
    }
}

fn print_flag_list(flags: &[FeatureFlag]) {
    if flags.is_empty() {
        println!("No feature flags found.");
        return;
    }
    let rows: Vec<FlagRow> = flags.iter().map(flag_to_row).collect();
    println!("{}", Table::new(rows));
}

fn print_flag_detail(flag: &FeatureFlag) -> Result<()> {
    println!("Key:     {}", flag.key);
    println!("Name:    {}", flag.name);
    println!("ID:      {}", flag.id);
    println!("Active:  {}", flag.active);
    println!("Created: {}", flag.created_at);
    println!("Filters: {}", serde_json::to_string_pretty(&flag.filters)?);
    Ok(())
}

fn run_flags(client: &PostHogClient, command: FlagsCommands) -> Result<()> {
    match command {
        FlagsCommands::List => {
            let flags = client.list_flags()?;
            print_flag_list(&flags);
        }
        FlagsCommands::Get { key } => {
            let flag = client.get_flag_by_key(&key)?;
            print_flag_detail(&flag)?;
        }
        FlagsCommands::Create { key } => {
            let flag = client.create_flag(&key)?;
            println!("Created feature flag '{}' (id: {})", flag.key, flag.id);
        }
        FlagsCommands::Enable { key } => {
            let flag = client.set_flag_active(&key, true)?;
            println!("Enabled feature flag '{}'", flag.key);
        }
        FlagsCommands::Disable { key } => {
            let flag = client.set_flag_active(&key, false)?;
            println!("Disabled feature flag '{}'", flag.key);
        }
        FlagsCommands::Delete { key } => {
            client.delete_flag(&key)?;
            println!("Deleted feature flag '{}'", key);
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load()?;
    let client = PostHogClient::new(&config)?;

    match cli.command {
        Commands::Flags { command } => run_flags(&client, command),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener};
    use std::sync::{Arc, Mutex};
    use std::thread;

    struct MockPostHog {
        base_url: String,
        requests: Arc<Mutex<Vec<String>>>,
    }

    impl MockPostHog {
        fn start() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let server_requests = Arc::clone(&requests);

            thread::spawn(move || {
                for stream in listener.incoming() {
                    let mut stream = stream.unwrap();
                    let mut buffer = [0_u8; 8192];
                    let bytes_read = stream.read(&mut buffer).unwrap();
                    let request = String::from_utf8_lossy(&buffer[..bytes_read]).to_string();
                    let request_line = request.lines().next().unwrap_or_default();
                    let body = response_body(request_line);
                    let status = response_status(request_line);
                    server_requests.lock().unwrap().push(request);
                    let response = format!(
                        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    stream.write_all(response.as_bytes()).unwrap();
                }
            });

            Self {
                base_url: format!("http://{}", display_addr(addr)),
                requests,
            }
        }

        fn client(&self) -> PostHogClient {
            PostHogClient::new(&Config {
                api_key: "phx_test".to_string(),
                host: self.base_url.clone(),
                project_id: Some(7),
            })
            .unwrap()
        }

        fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    fn display_addr(addr: SocketAddr) -> String {
        format!("{}:{}", addr.ip(), addr.port())
    }

    fn response_status(request_line: &str) -> &'static str {
        if request_line.starts_with("POST /api/projects/7/feature_flags/") {
            return "201 Created";
        }
        if request_line.starts_with("DELETE /api/projects/7/feature_flags/11/") {
            return "204 No Content";
        }
        "200 OK"
    }

    fn response_body(request_line: &str) -> &'static str {
        if request_line.starts_with("GET /api/projects/7/feature_flags/") {
            return r#"{"results":[{"id":11,"key":"flag-key","name":"Flag Name","active":false,"deleted":false,"filters":{"groups":[]},"created_at":"2026-02-03T04:05:06Z"}]}"#;
        }
        if request_line.starts_with("POST /api/projects/7/feature_flags/") {
            return r#"{"id":12,"key":"new-flag","name":"new-flag","active":true,"deleted":false,"filters":{"groups":[]},"created_at":"2026-02-04T00:00:00Z"}"#;
        }
        if request_line.starts_with("PATCH /api/projects/7/feature_flags/11/") {
            return r#"{"id":11,"key":"flag-key","name":"Flag Name","active":true,"deleted":false,"filters":{"groups":[]},"created_at":"2026-02-03T04:05:06Z"}"#;
        }
        ""
    }

    fn sample_flag(created_at: &str, active: bool) -> FeatureFlag {
        FeatureFlag {
            id: 7,
            key: "flag-key".to_string(),
            name: "Flag Name".to_string(),
            active,
            deleted: false,
            filters: json!({"groups": [{"rollout_percentage": 100}]}),
            created_at: created_at.to_string(),
        }
    }

    #[test]
    fn parses_flag_subcommands() {
        assert!(matches!(
            Cli::try_parse_from(["posthog", "flags", "list"])
                .unwrap()
                .command,
            Commands::Flags {
                command: FlagsCommands::List
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["posthog", "flags", "enable", "flag-key"])
                .unwrap()
                .command,
            Commands::Flags {
                command: FlagsCommands::Enable { key }
            } if key == "flag-key"
        ));
        assert!(matches!(
            Cli::try_parse_from(["posthog", "flags", "delete", "flag-key"])
                .unwrap()
                .command,
            Commands::Flags {
                command: FlagsCommands::Delete { key }
            } if key == "flag-key"
        ));
    }

    #[test]
    fn flag_to_row_formats_date_and_active_state() {
        let active_row = flag_to_row(&sample_flag("2026-02-03T04:05:06Z", true));
        assert_eq!(active_row.key, "flag-key");
        assert_eq!(active_row.name, "Flag Name");
        assert_eq!(active_row.active, "yes");
        assert_eq!(active_row.created, "2026-02-03");

        let inactive_row = flag_to_row(&sample_flag("", false));
        assert_eq!(inactive_row.active, "no");
        assert_eq!(inactive_row.created, "-");
    }

    #[test]
    fn print_helpers_accept_empty_list_and_details() {
        print_flag_list(&[]);
        print_flag_list(&[sample_flag("2026-02-03T04:05:06Z", true)]);
        print_flag_detail(&sample_flag("2026-02-03T04:05:06Z", true)).unwrap();
    }

    #[test]
    fn run_flags_dispatches_each_subcommand() {
        let server = MockPostHog::start();
        let client = server.client();

        run_flags(&client, FlagsCommands::List).unwrap();
        run_flags(
            &client,
            FlagsCommands::Get {
                key: "flag-key".to_string(),
            },
        )
        .unwrap();
        run_flags(
            &client,
            FlagsCommands::Create {
                key: "new-flag".to_string(),
            },
        )
        .unwrap();
        run_flags(
            &client,
            FlagsCommands::Enable {
                key: "flag-key".to_string(),
            },
        )
        .unwrap();
        run_flags(
            &client,
            FlagsCommands::Disable {
                key: "flag-key".to_string(),
            },
        )
        .unwrap();
        run_flags(
            &client,
            FlagsCommands::Delete {
                key: "flag-key".to_string(),
            },
        )
        .unwrap();

        let requests = server.requests();
        assert!(
            requests
                .iter()
                .any(|request| request.starts_with("POST /api/projects/7/feature_flags/"))
        );
        assert!(
            requests
                .iter()
                .any(|request| request.starts_with("PATCH /api/projects/7/feature_flags/11/"))
        );
        assert!(
            requests
                .iter()
                .any(|request| request.starts_with("DELETE /api/projects/7/feature_flags/11/"))
        );
    }
}
