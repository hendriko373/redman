use anyhow::Result;
use clap::{Parser, Subcommand};
use dotenv::dotenv;
use colored::*;
use redman::{fetch_data, get_torrents_not_in_plex_library, get_plex_library_albums, transform_groups, Database, GroupData, Type};
use url::Url;


#[derive(Parser)]
#[command(author, version, about = "Fetch and manage torrent collections", long_about = None)]
struct Args {
    /// Base URL for the tracker API
    #[arg(short, long, default_value = "https://redacted.sh/", global = true)]
    base_url: String,
    
    /// Database file path
    #[arg(short, long, default_value = "red_download_pool.db", global = true)]
    database: String,

    #[command(subcommand)]
    command: Commands,
}


#[derive(Subcommand)]
enum Commands {
    /// Fetch collage data from API and store in database
    Fetch {
        /// The type of the group to be fetched
        #[arg(value_enum)]
        ftype: Type,
        /// Collage or artist ID to fetch
        id: u32,
        #[arg(short, long, default_value = "10")]
        weight: u32,
        /// Show verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    Watch {
        /// The number of torrents to add to the watchlist
        #[arg(default_value = "2")]
        number: u32,
        #[arg(default_value = "com.plexapp.plugins.library.db")]
        plex_db: String,
    },
    /// Show statistics about stored data
    Stats,
}


#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    dotenv().ok();

    // Validate base URL
    if let Err(_) = Url::parse(&args.base_url) {
        eprintln!("{}", "Error: Invalid base URL provided".red());
        std::process::exit(1);
    }

    let db = Database::new(&args.database)?;

    match args.command {
        Commands::Fetch { id, ftype, weight, verbose } => {
            println!("{} collage {}...", "Fetching".green().bold(), id.to_string().cyan());
            
            let api_key = std::env::var("API_KEY").expect("API key environment variable not set");
            match fetch_data(&api_key, &args.base_url, id, ftype, verbose).await {
                Ok(group_data) => {
                    match group_data {
                        GroupData::CollageData(ref collage_data) => {
                            if verbose {
                                println!("{}: {}", "Collage name".cyan(), collage_data.name.bright_white());
                                println!("{}: {}", "Category".cyan(), collage_data.collage_category_name);
                                println!("{}: {}", "Total groups".cyan(), collage_data.torrent_groups.len());
                            }

                        },
                        GroupData::ArtistData(ref artist_data) => {
                            if verbose {
                                println!("{}: {}", "Artist name".cyan(), artist_data.name.bright_white());
                                println!("{}: {}", "Total groups".cyan(), artist_data.torrent_groups.len());
                            }
                        }
                    }
                    let groups = transform_groups(&group_data, weight);

                    match db.store_data(&groups) {
                        Ok(stored_count) => {
                            println!("{} {} torrents stored successfully!", 
                                "✓".green().bold(), stored_count.to_string().bright_white());
                        }
                        Err(e) => {
                            eprintln!("{} Failed to store data: {}", "✗".red().bold(), e);
                            std::process::exit(1);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("{} Failed to fetch : {}", "✗".red().bold(), e);
                    std::process::exit(1);
                }
            }
        },
        Commands::Watch { number: _, plex_db } => {
            let albums = get_plex_library_albums(&plex_db)?;
            println!("\n{} {} albums found in Plex library", 
                "✓".green().bold(), albums.len().to_string().bright_white());
            let torrs = get_torrents_not_in_plex_library(&args.database, &plex_db);
            match torrs {
                Ok(torrents) => {
                    if torrents.is_empty() {
                        println!("{} No torrents found that are not in Plex library", "✓".green().bold());
                    } else {
                        println!("\n{} {} torrents not found in Plex library:", 
                            "✓".green().bold(), torrents.len().to_string().bright_white());
                        for torrent in torrents {
                            println!("  - {} by {}", torrent.album_name.bright_white(), torrent.artist_names.bright_white());
                        }
                    }
                }
                Err(e) => {
                    eprintln!("{} Failed to get torrents: {}", "✗".red().bold(), e);
                    std::process::exit(1);
                }
            }
        },
        Commands::Stats => {
            match db.get_stats() {
                Ok(stats) => {
                    println!("\n{}", "Database Statistics".cyan().bold().underline());
                    println!("{}: {}", "Total Torrents".bold(), stats.total_torrents.to_string().bright_white());
                    println!("{}: {}", "Unique Artists".bold(), stats.unique_artists.to_string().bright_white());
                    println!("{}: {}", "Unique Albums".bold(), stats.unique_albums.to_string().bright_white());
                    
                    println!("\n{}", "Format Distribution:".bold());
                    for (format, count) in stats.format_counts {
                        let percentage = (count as f64 / stats.total_torrents as f64) * 100.0;
                        println!("  {}: {} ({:.1}%)", 
                            format.bright_white(), 
                            count.to_string().cyan(), 
                            percentage
                        );
                    }
                }
                Err(e) => {
                    eprintln!("{} Failed to get stats: {}", "✗".red().bold(), e);
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}
