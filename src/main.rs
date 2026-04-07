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
        active: if f.active { "yes".to_string() } else { "no".to_string() },
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
