#!/usr/bin/env python3
"""
Basic test script for the Python bindings.
This doesn't require actual Last.fm credentials, just tests object creation.
"""

try:
    # Test importing the module
    from python.lastfm_edit import (
        LastFmEditClient,
        Track,
        Album,
        Artist,
        ScrobbleEdit,
        EditResponse,
        LastFmEditSession,
    )
    print("✅ Successfully imported all classes")

    # Test creating objects
    track = Track(
        name="Test Track",
        artist="Test Artist",
        playcount=10,
        timestamp=None,
        album="Test Album",
        album_artist=None
    )
    print(f"✅ Created Track: {track}")

    album = Album(
        name="Test Album",
        artist="Test Artist", 
        playcount=50
    )
    print(f"✅ Created Album: {album}")

    artist = Artist(
        name="Test Artist",
        playcount=100
    )
    print(f"✅ Created Artist: {artist}")

    # Test ScrobbleEdit creation
    edit = ScrobbleEdit.for_artist("Old Artist", "New Artist")
    print(f"✅ Created ScrobbleEdit: {edit}")

    edit2 = ScrobbleEdit.from_track_and_artist("Track Name", "Artist Name")
    print(f"✅ Created ScrobbleEdit from track/artist: {edit2}")

    # Test client creation (doesn't require login)
    client = LastFmEditClient()
    print("✅ Created LastFmEditClient")

    print("\n🎉 All basic tests passed! The Python bindings are working correctly.")
    print("\nNote: To test actual functionality, you would need to call client.login() with valid credentials.")

except ImportError as e:
    print(f"❌ Import error: {e}")
    print("Make sure the module is compiled and available in the Python path")
except Exception as e:
    print(f"❌ Error: {e}")
    import traceback
    traceback.print_exc()