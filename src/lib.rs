use std::{
    collections::HashSet,
    fs::{self, File, remove_file},
    io::copy,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

use anyhow::Result;
use clap::ValueEnum;
use colored::*;
use html_escape::decode_html_entities;
use itertools::Itertools;
use rand::seq::SliceRandom;
use regex::Regex;
use reqwest::Client;
use rusqlite::{Connection, OpenFlags, params};
use serde::Deserialize;

#[derive(ValueEnum, Clone, Debug)]
pub enum Type {
    Collage,
    Artist,
}

impl std::fmt::Display for Type {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Type::Collage => write!(f, "collage"),
            Type::Artist => write!(f, "artist"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ApiResponseCollage {
    status: String,
    response: CollageData,
}

#[derive(Debug, Deserialize)]
struct ApiResponseArtist {
    status: String,
    response: ArtistData,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    status: String,
    response: GroupData,
}

#[derive(Debug, Deserialize)]
struct ApiResponseTorrent {
    response: TorrentResponse,
}

#[derive(Debug, Deserialize)]
struct TorrentResponse {
    torrent: TorrentData,
}

#[derive(Debug, Deserialize)]
struct TorrentData {
    #[serde(rename = "isFreeload")]
    is_freeload: bool,
}

#[derive(Debug, Deserialize)]
pub struct CollageData {
    pub id: u32,
    pub name: String,
    #[serde(rename = "collageCategoryName")]
    pub collage_category_name: String,
    #[serde(rename = "torrentgroups")]
    pub torrent_groups: Vec<TorrentGroupCollage>,
}

#[derive(Debug, Deserialize)]
pub struct ArtistData {
    pub id: u32,
    pub name: String,
    #[serde(alias = "torrentgroup")]
    pub torrent_groups: Vec<TorrentGroupArtist>,
}

#[derive(Debug, Deserialize)]
pub enum GroupData {
    CollageData(CollageData),
    ArtistData(ArtistData),
}

#[derive(Debug, Deserialize)]
pub struct TorrentGroupCollage {
    name: String,
    year: String,
    #[serde(alias = "releaseType")]
    release_type: String,
    #[serde(rename = "musicInfo")]
    music_info: MusicInfo,
    torrents: Vec<TorrentApi>,
}

#[derive(Debug, Deserialize)]
pub struct TorrentGroupArtist {
    #[serde(alias = "groupName")]
    name: String,
    #[serde(alias = "groupYear")]
    year: u32,
    #[serde(alias = "releaseType")]
    release_type: u32,
    #[serde(alias = "torrent")]
    torrents: Vec<TorrentApi>,
}

#[derive(Debug, Deserialize)]
struct MusicInfo {
    artists: Vec<Artist>,
}

#[derive(Debug, Deserialize)]
struct Artist {
    name: String,
}

#[derive(Debug, Deserialize)]
struct TorrentApi {
    #[serde(alias = "torrentid", alias = "id")]
    torrent_id: u32,
    media: String,
    format: String,
    encoding: String,
    #[serde(rename = "fileCount")]
    file_count: u32,
    size: u64,
}

#[derive(Debug, Clone)]
pub struct Torrent {
    pub id: u32,
    pub album_name: String,
    pub artist_names: String,
    year: u32,
    release_type: u32,
    media: String,
    format: String,
    encoding: String,
    file_count: u32,
    size: u64,
    weight: u32,
}

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn new(db_path: &str) -> Result<Self> {
        let db_exists = Path::new(db_path).exists();
        let conn = Connection::open(db_path)?;

        if !db_exists {
            println!("{}", "Creating new database...".green());
        }

        conn.execute(
            r#"
            CREATE TABLE IF NOT EXISTS torrents (
                id INTEGER PRIMARY KEY,
                album_name TEXT NOT NULL,
                artist_names TEXT NOT NULL,
                year INTEGER NOT NULL,
                release_type INTEGER NOT NULL,
                media TEXT NOT NULL,
                format TEXT NOT NULL,
                encoding TEXT NOT NULL,
                file_count INTEGER NOT NULL,
                size_bytes INTEGER NOT NULL,
                weight INTEGER NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )
            "#,
            [],
        )?;
        conn.execute(
            r#"
            CREATE TABLE IF NOT EXISTS fetches (
                id INTEGER NOT NULL,
                type INTEGER NOT NULL,
                name TEXT NOT NULL,
                created_at datetime DEFAULT CURRENT_TIMESTAMP,
                PRIMARY KEY (id, type)
            )
            "#,
            [],
        )?;

        Ok(Self { conn })
    }

    pub fn store_data(&self, group_data: &GroupData, weight: u32) -> Result<u32> {
        let mut stored_count = 0;

        self.conn.execute(
            r#"
            INSERT INTO fetches (id, type, name) VALUES (?, ?, ?) ON CONFLICT(id, type) DO NOTHING 
            "#,
            params![
                match group_data {
                    GroupData::ArtistData(a) => a.id,
                    GroupData::CollageData(c) => c.id,
                },
                match group_data {
                    GroupData::ArtistData(_) => 0,
                    GroupData::CollageData(_) => 1,
                },
                match group_data {
                    GroupData::ArtistData(a) => &a.name,
                    GroupData::CollageData(c) => &c.name,
                }
            ],
        )?;

        let groups = transform_groups(&group_data, weight);
        for g in groups {
            let mut torrents = g
                .iter()
                .filter(|t| t.release_type == 1)
                .filter(|t| {
                    (t.media == "CD" || t.media == "WEB")
                        && t.format == "MP3"
                        && (t.encoding == "V0 (VBR)" || t.encoding == "320")
                })
                .collect::<Vec<_>>();
            torrents.sort_by_key(|t| match (t.media.as_str(), t.encoding.as_str()) {
                ("CD", "V0 (VBR)") => 0,
                ("WEB", "V0 (VBR)") => 1,
                ("CD", "320") => 2,
                ("WEB", "320") => 3,
                _ => 99,
            });
            let torrent = torrents.first();
            if torrent.is_some() {
                let t = torrent.unwrap();

                let result = self.conn.execute(
                    r#"
                    INSERT OR REPLACE INTO torrents (
                        id, 
                        album_name, 
                        artist_names,
                        year, 
                        release_type,
                        media, 
                        format, 
                        encoding, 
                        file_count,
                        weight, 
                        size_bytes 
                    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    "#,
                    params![
                        t.id,
                        t.album_name,
                        t.artist_names,
                        t.year,
                        t.release_type,
                        t.media,
                        t.format,
                        t.encoding,
                        t.file_count,
                        t.weight,
                        t.size as i64,
                    ],
                )?;

                if result > 0 {
                    stored_count += 1;
                }
            }
        }

        Ok(stored_count)
    }

    pub fn get_stats(&self) -> Result<DatabaseStats> {
        let total_torrents: i64 =
            self.conn
                .query_row("SELECT COUNT(*) FROM torrents", [], |row| row.get(0))?;

        let unique_artists: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT artist_names) FROM torrents",
            [],
            |row| row.get(0),
        )?;

        let unique_albums: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT album_name) FROM torrents",
            [],
            |row| row.get(0),
        )?;

        let mut stmt = self.conn.prepare(
            "SELECT format, COUNT(*) as count FROM torrents GROUP BY format ORDER BY count DESC",
        )?;
        let format_counts_iter = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?;

        let mut format_counts = Vec::new();
        for fc in format_counts_iter {
            format_counts.push(fc?);
        }

        Ok(DatabaseStats {
            total_torrents,
            unique_artists,
            unique_albums,
            format_counts,
        })
    }
}

#[derive(Debug)]
pub struct DatabaseStats {
    pub total_torrents: i64,
    pub unique_artists: i64,
    pub unique_albums: i64,
    pub format_counts: Vec<(String, i64)>,
}

pub async fn fetch_data(
    api: &str,
    base_url: &str,
    id: u32,
    ftype: Type,
    verbose: bool,
) -> Result<GroupData> {
    let client = Client::new();
    let url = match ftype {
        Type::Artist => format!(
            "{}ajax.php?action=artist&id={}&artistreleases=1",
            base_url, id
        ),
        Type::Collage => format!("{}ajax.php?action=collage&id={}", base_url, id),
    };

    if verbose {
        println!("{} {}", "Fetching from:".cyan(), url.bright_blue());
    }

    let response = client.get(&url).header("Authorization", api).send().await?;

    if verbose {
        println!("{} {}", "Response status:".cyan(), response.status());
    }

    let api_response: ApiResponse = match ftype {
        Type::Artist => {
            let r = response.json::<ApiResponseArtist>().await?;
            ApiResponse {
                status: r.status,
                response: GroupData::ArtistData(r.response),
            }
        }
        Type::Collage => {
            let r = response.json::<ApiResponseCollage>().await?;
            ApiResponse {
                status: r.status,
                response: GroupData::CollageData(r.response),
            }
        }
    };
    if api_response.status != "success" {
        return Err(anyhow::anyhow!(
            "API returned error status: {}",
            api_response.status
        ));
    }

    Ok(api_response.response)
}

fn transform_groups(groups: &GroupData, weight: u32) -> Vec<Vec<Torrent>> {
    match groups {
        GroupData::ArtistData(artist) => artist
            .torrent_groups
            .iter()
            .map(|g| {
                g.torrents
                    .iter()
                    .map(|t| {
                        let artist_name = decode_html_entities(&artist.name);
                        let album_name = decode_html_entities(&g.name);
                        Torrent {
                            id: t.torrent_id,
                            album_name: album_name.to_string(),
                            artist_names: artist_name.to_string(),
                            year: g.year,
                            release_type: g.release_type,
                            media: t.media.clone(),
                            format: t.format.clone(),
                            encoding: t.encoding.clone(),
                            file_count: t.file_count,
                            weight: weight,
                            size: t.size,
                        }
                    })
                    .collect()
            })
            .collect(),
        GroupData::CollageData(collage) => collage
            .torrent_groups
            .iter()
            .map(|g| {
                let artist_names = g
                    .music_info
                    .artists
                    .iter()
                    .map(|a| a.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                g.torrents
                    .iter()
                    .map(|t| Torrent {
                        id: t.torrent_id,
                        album_name: g.name.clone(),
                        artist_names: artist_names.clone(),
                        year: g.year.parse().unwrap(),
                        release_type: g.release_type.parse().unwrap(),
                        media: t.media.clone(),
                        format: t.format.clone(),
                        encoding: t.encoding.clone(),
                        file_count: t.file_count,
                        weight: weight,
                        size: t.size,
                    })
                    .collect()
            })
            .collect(),
    }
}

pub async fn add_new_torrents_for_download(
    api: &str,
    base_url: &str,
    pool_db: &str,
    plex_db: &str,
    torrent_dir: &str,
    num_torrents: usize,
    remote_exe: &str,
    download_dir: &str,
    use_fl: bool,
    freeload_only: bool,
) -> Result<Vec<Torrent>> {
    let mut torrents = get_pool_torrents(pool_db)
        .and_then(|ts| filter_torrents_not_in_plex_library(&ts, plex_db))
        .and_then(|ts| filter_torrents_not_in_torrent_dir(&ts, torrent_dir))?;

    let mut groups: Vec<(u32, Vec<Torrent>)> = torrents
        .iter()
        .chunk_by(|t| t.weight)
        .into_iter()
        .map(|(w, group)| {
            let mut shuffled: Vec<Torrent> = group.cloned().collect();
            shuffled.shuffle(&mut rand::rng());
            (w, shuffled)
        })
        .collect();
    groups.sort_by_key(|t| t.0);
    groups.reverse();
    torrents = groups.into_iter().flat_map(|(_, group)| group).collect();

    if freeload_only {
        torrents = filter_freeload_torrents(&torrents, base_url, api, num_torrents).await?;
    } else {
        torrents = torrents.into_iter().take(num_torrents).collect::<Vec<_>>();
    }

    for t in &torrents {
        let path = download_torrent(t.id, base_url, api, torrent_dir, use_fl).await?;
        thread::sleep(Duration::from_millis(150)); // Do not spam redacted API
        let path_str = path.to_str().unwrap();
        let mut cmd = Command::new(remote_exe);
        cmd.arg("localhost:9091")
            .args(["-n", "transmission:transmission"])
            .args(["-a", path_str])
            .args(["--download-dir", download_dir])
            .arg("-s");
        let output = cmd.output();
        if output.is_err() {
            remove_file(&path)?;
            Err(anyhow::anyhow!(
                "{}: Could not add {} to transmission: {}",
                remote_exe,
                path_str,
                output.err().unwrap()
            ))?;
        }
    }
    Ok(torrents)
}

#[derive(Debug)]
struct Album {
    pub name: String,
    pub artists: String,
}

fn get_plex_library_albums(db_path: &str) -> Result<Vec<Album>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare(
        r#"
            SELECT DISTINCT b.title as album, c.title as artist
            from metadata_items a
            JOIN metadata_items b ON a.parent_id = b.id
            JOIN metadata_items c ON b.parent_id = c.id
            where b.metadata_type = 9 AND c.metadata_type = 8
        "#,
    )?;

    let r = stmt
        .query_map([], |row| {
            Ok(Album {
                name: row.get("album")?,
                artists: row.get("artist")?,
            })
        })?
        .filter(|res| res.is_ok())
        .map(|res| res.unwrap())
        .collect();
    Ok(r)
}

fn get_pool_torrents(db_path: &str) -> Result<Vec<Torrent>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare(
        r#"
            SELECT id, album_name, artist_names, year, release_type, media, format, encoding, file_count, weight, size_bytes
            FROM torrents
        "#)?;
    let r = stmt
        .query_map([], |row| {
            Ok(Torrent {
                id: row.get("id")?,
                album_name: row.get("album_name")?,
                artist_names: row.get("artist_names")?,
                year: row.get("year")?,
                release_type: row.get("release_type")?,
                media: row.get("media")?,
                format: row.get("format")?,
                encoding: row.get("encoding")?,
                file_count: row.get("file_count")?,
                weight: row.get("weight")?,
                size: row.get::<_, i64>("size_bytes")? as u64,
            })
        })?
        .map(|res| res.unwrap())
        .collect();
    Ok(r)
}

/// Get torrents from the download pool that are not in the Plex library
fn filter_torrents_not_in_plex_library(
    torrents: &Vec<Torrent>,
    plex_db: &str,
) -> Result<Vec<Torrent>> {
    let plex_albums = get_plex_library_albums(plex_db)?;

    let transform = |s: &String| {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_lowercase()
            .clone()
    };

    let filtered_torrents: Vec<Torrent> = torrents
        .into_iter()
        .filter(|t| {
            !plex_albums.iter().any(|a| {
                transform(&a.artists) == transform(&t.artist_names)
                    && transform(&a.name) == transform(&t.album_name)
            })
        })
        .cloned()
        .collect();

    Ok(filtered_torrents)
}

fn filter_torrents_not_in_torrent_dir(
    torrents: &Vec<Torrent>,
    torrent_dir: &str,
) -> Result<Vec<Torrent>> {
    let dir_torrent_ids = fs::read_dir(torrent_dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str().map(|s| s.to_owned())))
        .map(|s| {
            s.chars()
                .rev()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .chars()
                .rev()
                .collect::<String>()
        })
        .filter_map(|s| s.parse::<u32>().ok())
        .collect::<HashSet<_>>();

    Ok(torrents
        .iter()
        .filter(|t| !dir_torrent_ids.contains(&t.id))
        .cloned()
        .collect::<Vec<Torrent>>())
}

async fn filter_freeload_torrents(
    ts: &Vec<Torrent>,
    base_url: &str,
    api: &str,
    max_num: usize,
) -> Result<Vec<Torrent>> {
    let mut result = Vec::new();
    let client = Client::new();
    let mut i = 0;
    while result.len() < max_num && i < ts.len() {
        let t = &ts[i];
        let url = format!("{}ajax.php?action=torrent&id={}", base_url, t.id);
        let response = client.get(&url).header("Authorization", api).send().await?;
        thread::sleep(Duration::from_millis(150)); // Do not spam redacted API
        let r = response.json::<ApiResponseTorrent>().await?;
        if r.response.torrent.is_freeload {
            result.push(t.clone());
            println!("{} {}", "Freeload torrent added:".green(), t.id);
        } else {
            println!("{} {}", "Skipping non-freeload torrent:".yellow(), t.id);
        }
        i += 1;
    }
    Ok(result)
}

async fn download_torrent(
    torrent_id: u32,
    base_url: &str,
    api_key: &str,
    torrent_dir: &str,
    use_fl: bool,
) -> Result<PathBuf> {
    let client = Client::new();
    let response = request_torrent_download(&client, torrent_id, base_url, api_key, use_fl).await?;

    if response.status().is_success() {
        write_torrent(torrent_dir, response).await
    } else {
        thread::sleep(Duration::from_millis(150)); // Do not spam redacted API
        let response_no_fl =
            request_torrent_download(&client, torrent_id, base_url, api_key, false).await?;
        if response_no_fl.status().is_success() {
            write_torrent(torrent_dir, response_no_fl).await
        } else {
            Err(anyhow::anyhow!(
                "Error downloading torrent file: {}",
                response_no_fl.status()
            ))
        }
    }
}

async fn request_torrent_download(
    client: &Client,
    torrent_id: u32,
    base_url: &str,
    api_key: &str,
    use_fl: bool,
) -> Result<reqwest::Response, anyhow::Error> {
    let t = if use_fl { 1 } else { 0 };
    let url = format!(
        "{}ajax.php?action=download&id={}&usetoken={}",
        base_url, torrent_id, t
    );
    let response = client
        .get(&url)
        .header("Authorization", api_key)
        .send()
        .await?;
    Ok(response)
}

async fn write_torrent(
    torrent_dir: &str,
    response: reqwest::Response,
) -> std::result::Result<PathBuf, anyhow::Error> {
    let content = response
        .headers()
        .get("Content-disposition")
        .ok_or(anyhow::anyhow!(
            "Headers does not contain Content-disposition"
        ))
        .and_then(|c| {
            String::from_utf8(c.as_bytes().to_vec())
                .map_err(|e| anyhow::anyhow!("Invalid header value: {}", e))
        })?;
    let re = Regex::new(r#"filename="([^"]+)""#)?;
    let fname = re
        .captures(&content)
        .and_then(|caps| caps.get(1).map(|n| n.as_str().to_string()))
        .ok_or(anyhow::anyhow!(
            "Could not parse default torrent file name for {}",
            content
        ))?;
    let path = PathBuf::from(torrent_dir).join(fname);
    let mut file = File::create(path.clone())?;
    let bytes = response.bytes().await?;
    let mut content = bytes.as_ref();
    copy(&mut content, &mut file)?;
    Ok(path)
}
