"""
Python bindings for lastfm-edit Rust crate.

This library provides programmatic access to Last.fm's scrobble editing functionality
via web scraping using a Rust backend with Python bindings.

Key Features:
- Authentication with Last.fm username/password
- Library browsing (tracks, albums, artists)
- Bulk scrobble editing
- Search functionality
- Recent scrobbles monitoring

Basic Usage:
    from lastfm_edit import LastFmEditClient, ScrobbleEdit

    # Create client and login
    client = LastFmEditClient()
    client.login("your_username", "your_password")

    # Browse your library
    tracks = client.get_artist_tracks("Radiohead", limit=10)
    for track in tracks:
        print(f"{track.artist} - {track.name}")

    # Edit a scrobble
    edit = ScrobbleEdit.for_artist("Old Artist Name", "New Artist Name")
    response = client.edit_scrobble(edit)
    print(f"Edit successful: {response.success()}")
"""

from ._lastfm_edit import (
    PyLastFmEditClient as LastFmEditClient,
    PyTrack as Track,
    PyAlbum as Album,
    PyArtist as Artist,
    PyScrobbleEdit as ScrobbleEdit,
    PyEditResponse as EditResponse,
    PyLastFmEditSession as LastFmEditSession,
)

__version__ = "4.0.0"

__all__ = [
    "LastFmEditClient",
    "Track",
    "Album",
    "Artist",
    "ScrobbleEdit",
    "EditResponse",
    "LastFmEditSession",
]
