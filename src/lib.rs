use std::path::Path;

use anyhow::Result;
use clap::ValueEnum;
use reqwest::Client;
use colored::*;
use serde::Deserialize;
use rusqlite::{params, Connection, OpenFlags};


#[derive(ValueEnum, Clone, Debug)]
pub enum Type {
    Collage,
    Artist,
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
pub struct CollageData {
    pub name: String,
    #[serde(rename = "collageCategoryName")]
    pub collage_category_name: String,
    #[serde(rename = "torrentgroups")]
    pub torrent_groups: Vec<TorrentGroupCollage>,
}

#[derive(Debug, Deserialize)]
pub struct ArtistData {
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

pub struct Torrent {
    id: u32,
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

        Ok(Self { conn })
    }

    pub fn store_data(&self, groups: &Vec<Vec<Torrent>>) -> Result<u32> {
        let mut stored_count = 0;

        for group in groups {
            let mut torrents = group
                .iter()
                .filter(|t| t.release_type == 1)
                .filter(|t| 
                    (t.media == "CD" || t.media == "WEB") 
                    && t.format == "MP3"
                    && (t.encoding == "V0 (VBR)" || t.encoding == "320"))
                .collect::<Vec<_>>();
            torrents.sort_by_key(|t| {
                match (t.media.as_str(), t.encoding.as_str()) {
                    ("CD", "V0 (VBR)") => 0,
                    ("WEB", "V0 (VBR)") => 1,
                    ("CD", "320") => 2,
                    ("WEB", "320") => 3,
                    _ => 99
                }
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
        let total_torrents: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM torrents",
            [],
            |row| row.get(0),
        )?;

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
            "SELECT format, COUNT(*) as count FROM torrents GROUP BY format ORDER BY count DESC"
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

pub async fn fetch_data(api: &str, base_url: &str, id: u32, ftype: Type, verbose: bool) -> Result<GroupData> {
    let client = Client::new();
    let url = match ftype {
        Type::Artist => format!("{}ajax.php?action=artist&id={}&artistreleases=1", base_url, id),
        Type::Collage => format!("{}ajax.php?action=collage&id={}", base_url, id)
    };
    
    if verbose {
        println!("{} {}", "Fetching from:".cyan(), url.bright_blue());
    }

    let response = client.get(&url)
        .header("Authorization", api)
        .send()
        .await?;
    
    if verbose {
        println!("{} {}", "Response status:".cyan(), response.status());
    }

    let api_response: ApiResponse = match ftype {
        Type::Artist => {
            let r = response.json::<ApiResponseArtist>().await?;
            ApiResponse { status: r.status, response: GroupData::ArtistData(r.response) }
        },
        Type::Collage => {
            let r = response.json::<ApiResponseCollage>().await?;
            ApiResponse { status: r.status, response: GroupData::CollageData(r.response) }
        },
    };
    if api_response.status != "success" {
        return Err(anyhow::anyhow!("API returned error status: {}", api_response.status));
    }

    Ok(api_response.response)
}

pub fn transform_groups(groups: &GroupData, weight: u32) -> Vec<Vec<Torrent>> {
    match groups {
        GroupData::ArtistData(artist) => {
            artist.torrent_groups.iter()
                .map(|g| g.torrents.iter()
                    .map(|t| Torrent {
                        id: t.torrent_id,
                        album_name: g.name.clone(),
                        artist_names: artist.name.clone(),
                        year: g.year,
                        release_type: g.release_type,
                        media: t.media.clone(),
                        format: t.format.clone(),
                        encoding: t.encoding.clone(),
                        file_count: t.file_count,
                        weight: weight,
                        size: t.size,
                    }).collect())
                .collect()
        },
        GroupData::CollageData(collage) => {
            collage.torrent_groups.iter()
                .map(|g| {
                    let artist_names = g.music_info.artists
                            .iter()
                            .map(|a| a.name.clone())
                            .collect::<Vec<_>>()
                            .join(", ");
                    g.torrents.iter()
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
                        }).collect()
                }).collect()
        },
    }
}


#[derive(Debug)]
pub struct Album {
    pub name: String,
    pub artists: String,
}

pub fn get_plex_library_albums(db_path: &str) -> Result<Vec<Album>> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = conn.prepare(
        r#"
            SELECT DISTINCT b.title as album, c.title as artist
            from metadata_items a
            JOIN metadata_items b ON a.parent_id = b.id
            JOIN metadata_items c ON b.parent_id = c.id
            where b.metadata_type = 9 AND c.metadata_type = 8
        "#)?;
    let r = stmt.query_map([], |row| {
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

pub fn get_pool_torrents(db_path: &str) -> Result<Vec<Torrent>> {
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
pub fn get_torrents_not_in_plex_library(db_path: &str, plex_db: &str) -> Result<Vec<Torrent>> {
    let pool_torrents = get_pool_torrents(db_path)?;
    let plex_albums = get_plex_library_albums(plex_db)?;

    let plex_album_names: Vec<String> = plex_albums.iter()
        .map(|a| a.name.to_lowercase().clone()).collect();
    let plex_artist_names: Vec<String> = plex_albums.iter()
        .map(|a| a.artists.to_lowercase().clone()).collect();

    let filtered_torrents: Vec<Torrent> = pool_torrents.into_iter()
        .filter(|t| {
            !plex_album_names.contains(&t.album_name.to_lowercase()) 
                || !plex_artist_names.contains(&t.artist_names.to_lowercase())
        })
        .collect();

    Ok(filtered_torrents)
}

pub async fn download_torrent(torrent_id: u32, base_url: &str, api_key: &str) -> Result<Vec<u8>> {
    let client = Client::new();
    let url = format!("{}ajax.php?action=download&id={}", base_url, torrent_id);
    let response = client.get(&url)
        .header("Authorization", api_key)
        .send()
        .await?;
    
    if response.status().is_success() {
        let bytes = response.bytes().await?;
        Ok(bytes.to_vec())
    } else {
        Err(anyhow::anyhow!("Failed to download torrent: HTTP {}", response.status()))
    }
}