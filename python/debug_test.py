#!/usr/bin/env python3
"""
Simple test script to debug UTF-8 encoding issues in Python bindings
"""
import os
import sys

def main():
    try:
        # Check environment variables
        username = os.environ.get('LASTFM_EDIT_USERNAME')
        password = os.environ.get('LASTFM_EDIT_PASSWORD')
        
        if not username or not password:
            print("❌ Missing environment variables LASTFM_EDIT_USERNAME or LASTFM_EDIT_PASSWORD")
            return 1
            
        print(f"🔧 Testing Python bindings for user: {username}")
        
        # Import the module
        print("📦 Importing lastfm_edit module...")
        from lastfm_edit import LastFmEditClient
        
        # Create client
        print("🏗️  Creating client...")
        client = LastFmEditClient()
        
        # Login
        print("🔑 Logging in...")
        client.login(username, password)
        print(f"✅ Login successful as: {client.username()}")
        
        # Test validate_session (simplest call)
        print("🔍 Testing session validation...")
        is_valid = client.validate_session()
        print(f"✅ Session validation: {is_valid}")
        
        # Test get_session (simplest data access)
        print("📋 Testing get_session()...")
        try:
            session = client.get_session()
            print(f"✅ Session info: username='{session.username}', base_url='{session.base_url}'")
        except Exception as e:
            print(f"❌ get_session failed: {e}")
        
        # Test find_recent_scrobble_for_track (smaller search)
        print("🔎 Testing find_recent_scrobble_for_track...")
        try:
            track = client.find_recent_scrobble_for_track("test", "test", 1)
            print(f"✅ Search completed: {track}")
        except Exception as e:
            print(f"❌ find_recent_scrobble_for_track failed: {e}")
            print(f"   Error type: {type(e)}")
        
        # Test get_recent_scrobbles with page=1 (direct API call)
        print("📄 Testing get_recent_scrobbles(page=1)...")
        try:
            tracks = client.get_recent_scrobbles(1)
            print(f"✅ Got {len(tracks)} recent scrobbles from page 1")
            if tracks:
                track = tracks[0]
                print(f"   First track: {track.artist} - {track.name}")
        except Exception as e:
            print(f"❌ get_recent_scrobbles failed: {e}")
            print(f"   Error type: {type(e)}")
        
        # Test iterator-based methods
        print("🎵 Testing get_recent_tracks(limit=3)...")
        try:
            tracks = client.get_recent_tracks(limit=3)
            print(f"✅ Got {len(tracks)} recent tracks via iterator")
            for i, track in enumerate(tracks):
                print(f"   {i+1}. {track.artist} - {track.name}")
        except Exception as e:
            print(f"❌ get_recent_tracks failed: {e}")
            print(f"   Error type: {type(e)}")
        
        print("🎨 Testing get_artists(limit=3)...")
        try:
            artists = client.get_artists(limit=3)
            print(f"✅ Got {len(artists)} artists")
            for i, artist in enumerate(artists):
                print(f"   {i+1}. {artist.name} ({artist.playcount} plays)")
        except Exception as e:
            print(f"❌ get_artists failed: {e}")
            print(f"   Error type: {type(e)}")
            
        print("\n🎉 Test completed!")
        return 0
        
    except Exception as e:
        print(f"❌ Test failed with error: {e}")
        print(f"   Error type: {type(e)}")
        import traceback
        traceback.print_exc()
        return 1

if __name__ == "__main__":
    sys.exit(main())