# Usage

Use the CLI for fetching collages and artist torrents to a given pool sqlite database, as well as for downloading the fetched torrents for the transmission client. Before downloading, it is ensured the torrent candidates from the pool are not yet in the user's PLEX library or the transmission torrents folder.

```
Fetch and manage torrent collections

Usage: redman [OPTIONS] --pool <POOL> <COMMAND>

Commands:
  fetch     Fetch collage data from API and store in database
  download  Add torrents not in library to the transmission client for download
  stats     Show statistics about stored data
  help      Print this message or the help of the given subcommand(s)

Options:
  -b, --base-url <BASE_URL>  Base URL for the tracker API [default: https://redacted.sh/]
  -p, --pool <POOL>          Database file path for storing torrent pool data
  -h, --help                 Print help
  -V, --version              Print version
```

# Build from source

## Synology ARM

```
cross build --release --target aarch64-unknown-linux-musl
```

Make sure Podman is installed.
