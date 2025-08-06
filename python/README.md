# lastfm-edit Python Bindings

Python bindings for the [lastfm-edit](https://github.com/colonelpanic8/lastfm-edit) Rust crate, providing programmatic access to Last.fm's scrobble editing functionality via web scraping.

## Features

- **Authentication**: Login to Last.fm with username/password
- **Library browsing**: Access tracks, albums, and recent scrobbles
- **Bulk editing**: Edit track names, artist names, and album information
- **Search functionality**: Search through your scrobbled music
- **Async support**: Built on a high-performance async Rust backend

## Installation

```bash
pip install lastfm-edit
```

## Quick Start

```python
from lastfm_edit import LastFmEditClient, ScrobbleEdit

# Create client and login
client = LastFmEditClient()
client.login("your_username", "your_password")

# Browse recent tracks
recent_tracks = client.get_recent_tracks(limit=10)
for track in recent_tracks:
    print(f"{track.artist} - {track.name}")

# Get all tracks by an artist
radiohead_tracks = client.get_artist_tracks("Radiohead", limit=50)
print(f"Found {len(radiohead_tracks)} Radiohead tracks")

# Edit scrobbles (rename an artist)
edit = ScrobbleEdit.for_artist("Old Artist Name", "New Artist Name")
response = client.edit_scrobble(edit)

if response.success():
    print(f"Successfully edited {response.successful_edits()} scrobbles")
else:
    print(f"Edit failed: {response.message()}")
```

## Core Classes

### LastFmEditClient

The main client for interacting with Last.fm:

```python
client = LastFmEditClient()
client.login("username", "password")

# Check if session is valid
if client.validate_session():
    print(f"Logged in as: {client.username()}")
```

### Track, Album, Artist

Data classes representing music metadata:

```python
# Track has: name, artist, playcount, timestamp, album, album_artist
track = Track("Song Title", "Artist Name", 42, None, "Album Name", None)
print(f"{track.artist} - {track.name} (played {track.playcount} times)")

# Album has: name, artist, playcount, timestamp  
album = Album("Album Name", "Artist Name", 100, None)

# Artist has: name, playcount, timestamp
artist = Artist("Artist Name", 500, None)
```

### ScrobbleEdit

Represents edit operations on scrobbles:

```python
# Edit all tracks by an artist
edit = ScrobbleEdit.for_artist("Old Name", "New Name")

# Edit a specific track
edit = ScrobbleEdit.from_track_and_artist("Track Name", "Artist Name")
edit = edit.with_track_name("New Track Name")

# Edit all tracks in an album
edit = ScrobbleEdit.for_album("Album Name", "Old Artist", "New Artist")
```

## Library Browsing

```python
# Get all artists (with optional limit)
artists = client.get_artists(limit=100)

# Get tracks by artist
tracks = client.get_artist_tracks("Radiohead")

# Get albums by artist  
albums = client.get_artist_albums("Radiohead")

# Get tracks from specific album
album_tracks = client.get_album_tracks("OK Computer", "Radiohead")

# Get recent scrobbles
recent = client.get_recent_tracks(limit=50)

# Search functionality
search_results = client.search_tracks("paranoid android")
album_results = client.search_albums("ok computer")
```

## Bulk Editing Examples

### Rename an Artist

```python
# Rename all scrobbles by "The Beatles" to "Beatles"
edit = ScrobbleEdit.for_artist("The Beatles", "Beatles")
response = client.edit_scrobble(edit)
print(f"Renamed {response.successful_edits()} scrobbles")
```

### Fix Track Names

```python
# Find tracks with "(Remaster)" in the name and remove it
tracks = client.get_artist_tracks("Pink Floyd")
for track in tracks:
    if "(Remaster)" in track.name:
        new_name = track.name.replace(" (Remaster)", "")
        edit = ScrobbleEdit.from_track_and_artist(track.name, track.artist)
        edit = edit.with_track_name(new_name)
        
        response = client.edit_scrobble(edit)
        if response.success():
            print(f"Fixed: {track.name} -> {new_name}")
```

### Album-wide Changes  

```python
# Change artist for all tracks in a specific album
edit = ScrobbleEdit.for_album("Sgt. Pepper's Lonely Hearts Club Band", 
                             "The Beatles", "Beatles")
response = client.edit_scrobble(edit)
```

## Error Handling

```python
try:
    client.login("username", "password")
except Exception as e:
    print(f"Login failed: {e}")

try:
    tracks = client.get_artist_tracks("Nonexistent Artist")
except Exception as e:
    print(f"Failed to get tracks: {e}")
```

## Development

The Python bindings are built using [maturin](https://github.com/PyO3/maturin) and [PyO3](https://github.com/PyO3/pyo3).

### Building from Source

```bash
# Install maturin
pip install maturin

# Clone the repository  
git clone https://github.com/colonelpanic8/lastfm-edit
cd lastfm-edit/python

# Build and install in development mode
maturin develop

# Or build wheel
maturin build --release
```

## License

MIT License - see the main repository for details.

## Related Projects

- [lastfm-edit](https://github.com/colonelpanic8/lastfm-edit) - The Rust crate these bindings wrap
- [Last.fm](https://last.fm) - The music tracking service