#!/usr/bin/env python3
"""
Startup script for IPython with authenticated LastFm client
"""
import os
from lastfm_edit import LastFmEditClient, Track, Album, Artist, ScrobbleEdit, EditResponse

print("🔑 Authenticating with Last.fm...")
client = LastFmEditClient()

try:
    username = os.environ['LASTFM_EDIT_USERNAME']
    password = os.environ['LASTFM_EDIT_PASSWORD']
    client.login(username, password)
    print(f"✅ Successfully authenticated as: {client.username()}")
    print()
    print("🚀 Ready to go! Try:")
    print("   tracks = client.get_recent_tracks(limit=5)")
    print("   artists = client.get_artists(limit=10)")
    print("   edit = ScrobbleEdit.for_artist('Old Name', 'New Name')")
    print("   # response = client.edit_scrobble(edit)")
    print()
except Exception as e:
    print(f"❌ Authentication failed: {e}")
    print("   The client object is still available but not authenticated")
    print()