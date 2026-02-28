use clap::Parser;

mod checkpoint;
mod claude_binary;
mod commands;
mod process;
mod web_server;

use commands::agents::{init_database_with_path, AgentDb};

#[derive(Parser)]
#[command(name = "VibeAgentTeamWeb")]
#[command(about = "Vibe Agent Team Web Server - Access from your phone")]
struct Args {
    /// Port to run the web server on
    #[arg(short, long, default_value = "8080")]
    port: u16,

    /// Host to bind to (0.0.0.0 for all interfaces)
    #[arg(short = 'H', long, default_value = "0.0.0.0")]
    host: String,
}

#[tokio::main]
async fn main() {
    env_logger::init();

    let args = Args::parse();

    println!("🚀 Starting Web Server...");
    println!(
        "📱 Will be accessible from phones at: http://{}:{}",
        args.host, args.port
    );

    // Initialize database (using a temporary app handle for web mode)
    let db_path = std::path::PathBuf::from("VibeAgentTeam.db");
    let conn = match init_database_with_path(&db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("❌ Failed to initialize database: {}", e);
            std::process::exit(1);
        }
    };
    let db = AgentDb(std::sync::Arc::new(std::sync::Mutex::new(conn)));

    if let Err(e) = web_server::start_web_mode(Some(args.port), db).await {
        eprintln!("❌ Failed to start web server: {}", e);
        std::process::exit(1);
    }
}
