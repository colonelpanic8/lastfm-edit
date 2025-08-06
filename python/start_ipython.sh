#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

# Check if environment variables are set
if [ -z "${LASTFM_EDIT_USERNAME:-}" ] || [ -z "${LASTFM_EDIT_PASSWORD:-}" ]; then
    echo "❌ Missing environment variables:"
    echo "   Please set LASTFM_EDIT_USERNAME and LASTFM_EDIT_PASSWORD"
    echo "   These should be available in your .envrc file"
    exit 1
fi

echo "🐍 Starting IPython session with authenticated LastFm client..."
echo "📦 Available objects:"
echo "   - client: Authenticated LastFmEditClient"
echo "   - Track, Album, Artist: Data classes"
echo "   - ScrobbleEdit: For creating edit operations"
echo

# Run IPython with startup script
uv run python -c "
import os
try:
    from lastfm_edit import LastFmEditClient, Track, Album, Artist, ScrobbleEdit, EditResponse
    
    print('🔑 Authenticating with Last.fm...')
    client = LastFmEditClient()
    
    username = os.environ['LASTFM_EDIT_USERNAME']
    password = os.environ['LASTFM_EDIT_PASSWORD']
    client.login(username, password)
    print(f'✅ Successfully authenticated as: {client.username()}')
    print()
    print('🚀 Ready to go! Try:')
    print('   tracks = client.get_recent_tracks(limit=5)')
    print('   artists = client.get_artists(limit=10)') 
    print('   edit = ScrobbleEdit.for_artist(\"Old Name\", \"New Name\")')
    print('   # response = client.edit_scrobble(edit)')
    print()
except Exception as e:
    print(f'❌ Setup failed: {e}')
    print('   Will start IPython without pre-configured client')
    print()

# Start IPython with the loaded modules in namespace
import IPython
IPython.start_ipython(argv=[], user_ns=locals())
"