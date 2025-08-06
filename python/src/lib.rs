use lastfm_edit::{
    Album, Artist, EditResponse,
    LastFmEditClient, LastFmEditClientImpl, LastFmEditSession,
    ScrobbleEdit, Track,
};
use pyo3::prelude::*;
use pyo3::types::{PyModule, PyType};
use std::sync::Arc;
use tokio::runtime::Runtime;

/// Python wrapper for the Last.fm Edit Client
#[pyclass]
pub struct PyLastFmEditClient {
    client: LastFmEditClientImpl,
    runtime: Arc<Runtime>,
}

/// Python wrapper for Track
#[pyclass]
#[derive(Clone)]
pub struct PyTrack {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub artist: String,
    #[pyo3(get)]
    pub playcount: u32,
    #[pyo3(get)]
    pub timestamp: Option<u64>,
    #[pyo3(get)]
    pub album: Option<String>,
    #[pyo3(get)]
    pub album_artist: Option<String>,
}

/// Python wrapper for Album
#[pyclass]
#[derive(Clone)]
pub struct PyAlbum {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub artist: String,
    #[pyo3(get)]
    pub playcount: u32,
    #[pyo3(get)]
    pub timestamp: Option<u64>,
}

/// Python wrapper for Artist
#[pyclass]
#[derive(Clone)]
pub struct PyArtist {
    #[pyo3(get)]
    pub name: String,
    #[pyo3(get)]
    pub playcount: u32,
    #[pyo3(get)]
    pub timestamp: Option<u64>,
}

/// Python wrapper for ScrobbleEdit
#[pyclass]
#[derive(Clone)]
pub struct PyScrobbleEdit {
    inner: ScrobbleEdit,
}

/// Python wrapper for EditResponse
#[pyclass]
pub struct PyEditResponse {
    inner: EditResponse,
}

/// Python wrapper for LastFmEditSession
#[pyclass]
pub struct PyLastFmEditSession {
    #[pyo3(get)]
    pub username: String,
    #[pyo3(get)]
    pub base_url: String,
}

impl From<Track> for PyTrack {
    fn from(track: Track) -> Self {
        Self {
            name: track.name,
            artist: track.artist,
            playcount: track.playcount,
            timestamp: track.timestamp,
            album: track.album,
            album_artist: track.album_artist,
        }
    }
}

impl From<Album> for PyAlbum {
    fn from(album: Album) -> Self {
        Self {
            name: album.name,
            artist: album.artist,
            playcount: album.playcount,
            timestamp: album.timestamp,
        }
    }
}

impl From<Artist> for PyArtist {
    fn from(artist: Artist) -> Self {
        Self {
            name: artist.name,
            playcount: artist.playcount,
            timestamp: artist.timestamp,
        }
    }
}

#[pymethods]
impl PyTrack {
    #[new]
    #[pyo3(signature = (name, artist, playcount, timestamp=None, album=None, album_artist=None))]
    fn new(
        name: String,
        artist: String,
        playcount: u32,
        timestamp: Option<u64>,
        album: Option<String>,
        album_artist: Option<String>,
    ) -> Self {
        Self {
            name,
            artist,
            playcount,
            timestamp,
            album,
            album_artist,
        }
    }

    fn __str__(&self) -> String {
        let album_part = if let Some(ref album) = self.album {
            format!(" [{}]", album)
        } else {
            String::new()
        };
        format!("{} - {}{}", self.artist, self.name, album_part)
    }

    fn __repr__(&self) -> String {
        format!(
            "PyTrack(name='{}', artist='{}', playcount={}, timestamp={:?}, album={:?}, album_artist={:?})",
            self.name, self.artist, self.playcount, self.timestamp, self.album, self.album_artist
        )
    }
}

#[pymethods]
impl PyAlbum {
    #[new]
    #[pyo3(signature = (name, artist, playcount, timestamp=None))]
    fn new(name: String, artist: String, playcount: u32, timestamp: Option<u64>) -> Self {
        Self {
            name,
            artist,
            playcount,
            timestamp,
        }
    }

    fn __str__(&self) -> String {
        format!("{} - {}", self.artist, self.name)
    }

    fn __repr__(&self) -> String {
        format!(
            "PyAlbum(name='{}', artist='{}', playcount={}, timestamp={:?})",
            self.name, self.artist, self.playcount, self.timestamp
        )
    }
}

#[pymethods]
impl PyArtist {
    #[new]
    #[pyo3(signature = (name, playcount, timestamp=None))]
    fn new(name: String, playcount: u32, timestamp: Option<u64>) -> Self {
        Self {
            name,
            playcount,
            timestamp,
        }
    }

    fn __str__(&self) -> String {
        self.name.clone()
    }

    fn __repr__(&self) -> String {
        format!(
            "PyArtist(name='{}', playcount={}, timestamp={:?})",
            self.name, self.playcount, self.timestamp
        )
    }
}

#[pymethods]
impl PyScrobbleEdit {
    #[new]
    #[pyo3(signature = (
        artist_name_original,
        artist_name,
        track_name_original = None,
        track_name = None,
        album_name_original = None,
        album_name = None,
        album_artist_name_original = None,
        album_artist_name = None,
        timestamp = None,
        edit_all = true
    ))]
    fn new(
        artist_name_original: String,
        artist_name: String,
        track_name_original: Option<String>,
        track_name: Option<String>,
        album_name_original: Option<String>,
        album_name: Option<String>,
        album_artist_name_original: Option<String>,
        album_artist_name: Option<String>,
        timestamp: Option<u64>,
        edit_all: bool,
    ) -> Self {
        let edit = ScrobbleEdit::new(
            track_name_original,
            album_name_original,
            artist_name_original,
            album_artist_name_original,
            track_name,
            album_name,
            artist_name,
            album_artist_name,
            timestamp,
            edit_all,
        );
        Self { inner: edit }
    }

    #[classmethod]
    fn from_track_info(
        _cls: &Bound<'_, PyType>,
        original_track: &str,
        original_album: &str,
        original_artist: &str,
        timestamp: u64,
    ) -> Self {
        let edit = ScrobbleEdit::from_track_info(
            original_track,
            original_album,
            original_artist,
            timestamp,
        );
        Self { inner: edit }
    }

    #[classmethod]
    fn from_track_and_artist(_cls: &Bound<'_, PyType>, track_name: &str, artist_name: &str) -> Self {
        let edit = ScrobbleEdit::from_track_and_artist(track_name, artist_name);
        Self { inner: edit }
    }

    #[classmethod]
    fn for_artist(_cls: &Bound<'_, PyType>, old_artist_name: &str, new_artist_name: &str) -> Self {
        let edit = ScrobbleEdit::for_artist(old_artist_name, new_artist_name);
        Self { inner: edit }
    }

    #[classmethod]
    fn for_album(
        _cls: &Bound<'_, PyType>,
        album_name: &str,
        old_artist_name: &str,
        new_artist_name: &str,
    ) -> Self {
        let edit = ScrobbleEdit::for_album(album_name, old_artist_name, new_artist_name);
        Self { inner: edit }
    }

    fn with_track_name(&mut self, track_name: &str) -> Self {
        let edit = self.inner.clone().with_track_name(track_name);
        Self { inner: edit }
    }

    fn with_album_name(&mut self, album_name: &str) -> Self {
        let edit = self.inner.clone().with_album_name(album_name);
        Self { inner: edit }
    }

    fn with_artist_name(&mut self, artist_name: &str) -> Self {
        let edit = self.inner.clone().with_artist_name(artist_name);
        Self { inner: edit }
    }

    fn with_edit_all(&mut self, edit_all: bool) -> Self {
        let edit = self.inner.clone().with_edit_all(edit_all);
        Self { inner: edit }
    }

    fn __str__(&self) -> String {
        format!("{}", self.inner)
    }

    fn __repr__(&self) -> String {
        format!("PyScrobbleEdit({})", self.inner)
    }

    #[getter]
    fn track_name_original(&self) -> Option<String> {
        self.inner.track_name_original.clone()
    }

    #[getter]
    fn album_name_original(&self) -> Option<String> {
        self.inner.album_name_original.clone()
    }

    #[getter]
    fn artist_name_original(&self) -> String {
        self.inner.artist_name_original.clone()
    }

    #[getter]
    fn album_artist_name_original(&self) -> Option<String> {
        self.inner.album_artist_name_original.clone()
    }

    #[getter]
    fn track_name(&self) -> Option<String> {
        self.inner.track_name.clone()
    }

    #[getter]
    fn album_name(&self) -> Option<String> {
        self.inner.album_name.clone()
    }

    #[getter]
    fn artist_name(&self) -> String {
        self.inner.artist_name.clone()
    }

    #[getter]
    fn album_artist_name(&self) -> Option<String> {
        self.inner.album_artist_name.clone()
    }

    #[getter]
    fn timestamp(&self) -> Option<u64> {
        self.inner.timestamp
    }

    #[getter]
    fn edit_all(&self) -> bool {
        self.inner.edit_all
    }
}

#[pymethods]
impl PyEditResponse {
    fn success(&self) -> bool {
        self.inner.success()
    }

    fn all_successful(&self) -> bool {
        self.inner.all_successful()
    }

    fn any_successful(&self) -> bool {
        self.inner.any_successful()
    }

    fn total_edits(&self) -> usize {
        self.inner.total_edits()
    }

    fn successful_edits(&self) -> usize {
        self.inner.successful_edits()
    }

    fn failed_edits(&self) -> usize {
        self.inner.failed_edits()
    }

    fn summary_message(&self) -> String {
        self.inner.summary_message()
    }

    fn detailed_messages(&self) -> Vec<String> {
        self.inner.detailed_messages()
    }

    fn message(&self) -> Option<String> {
        self.inner.message()
    }

    fn __str__(&self) -> String {
        self.inner.summary_message()
    }

    fn __repr__(&self) -> String {
        format!(
            "PyEditResponse(success={}, total_edits={})",
            self.success(),
            self.total_edits()
        )
    }
}

#[pymethods]
impl PyLastFmEditSession {
    #[new]
    #[pyo3(signature = (username, _cookies, _csrf_token=None, base_url="https://www.last.fm".to_string()))]
    fn new(
        username: String,
        _cookies: Vec<String>,
        _csrf_token: Option<String>,
        base_url: String,
    ) -> Self {
        Self { username, base_url }
    }

    fn __repr__(&self) -> String {
        format!(
            "PyLastFmEditSession(username='{}', base_url='{}')",
            self.username, self.base_url
        )
    }
}

#[pymethods]
impl PyLastFmEditClient {
    #[new]
    fn new() -> PyResult<Self> {
        let runtime = Arc::new(Runtime::new().map_err(|e| {
            PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
                "Failed to create async runtime: {}",
                e
            ))
        })?);

        // We'll need to initialize the client later with login
        // For now, create a placeholder that will be replaced
        let http_client = Box::new(http_client::native::NativeClient::new());
        let session = LastFmEditSession::new(
            "".to_string(),
            vec![],
            None,
            "https://www.last.fm".to_string(),
        );
        let client = LastFmEditClientImpl::from_session(http_client, session);

        Ok(Self { client, runtime })
    }

    /// Login with username and password
    #[pyo3(signature = (username, password, base_url = None))]
    fn login(&mut self, username: &str, password: &str, base_url: Option<&str>) -> PyResult<()> {
        let _base_url = base_url.unwrap_or("https://www.last.fm");

        let result = self.runtime.block_on(async {
            let http_client = Box::new(http_client::native::NativeClient::new());
            LastFmEditClientImpl::login_with_credentials(http_client, username, password).await
        });

        match result {
            Ok(client) => {
                self.client = client;
                Ok(())
            }
            Err(e) => Err(PyErr::new::<pyo3::exceptions::PyException, _>(format!(
                "Login failed: {}",
                e
            ))),
        }
    }

    /// Get the username of the logged-in user
    fn username(&self) -> String {
        self.client.username()
    }

    /// Validate the current session
    fn validate_session(&self) -> bool {
        self.runtime.block_on(self.client.validate_session())
    }

    /// Get recent scrobbles for a specific page
    fn get_recent_scrobbles(&self, page: u32) -> PyResult<Vec<PyTrack>> {
        let result = self
            .runtime
            .block_on(self.client.get_recent_scrobbles(page));
        match result {
            Ok(tracks) => Ok(tracks.into_iter().map(PyTrack::from).collect()),
            Err(e) => Err(PyErr::new::<pyo3::exceptions::PyException, _>(format!(
                "Failed to get recent scrobbles: {}",
                e
            ))),
        }
    }

    /// Get all tracks by an artist (returns up to limit tracks)
    #[pyo3(signature = (artist, limit = None))]
    fn get_artist_tracks(&self, artist: &str, limit: Option<usize>) -> PyResult<Vec<PyTrack>> {
        let result = self.runtime.block_on(async {
            let mut iterator = self.client.artist_tracks(artist);
            if let Some(limit) = limit {
                iterator.take(limit).await
            } else {
                iterator.collect_all().await
            }
        });

        match result {
            Ok(tracks) => Ok(tracks.into_iter().map(PyTrack::from).collect()),
            Err(e) => Err(PyErr::new::<pyo3::exceptions::PyException, _>(format!(
                "Failed to get artist tracks: {}",
                e
            ))),
        }
    }

    /// Get all albums by an artist (returns up to limit albums)
    #[pyo3(signature = (artist, limit = None))]
    fn get_artist_albums(&self, artist: &str, limit: Option<usize>) -> PyResult<Vec<PyAlbum>> {
        let result = self.runtime.block_on(async {
            let mut iterator = self.client.artist_albums(artist);
            if let Some(limit) = limit {
                iterator.take(limit).await
            } else {
                iterator.collect_all().await
            }
        });

        match result {
            Ok(albums) => Ok(albums.into_iter().map(PyAlbum::from).collect()),
            Err(e) => Err(PyErr::new::<pyo3::exceptions::PyException, _>(format!(
                "Failed to get artist albums: {}",
                e
            ))),
        }
    }

    /// Get all tracks from a specific album (returns up to limit tracks)
    #[pyo3(signature = (album_name, artist_name, limit = None))]
    fn get_album_tracks(
        &self,
        album_name: &str,
        artist_name: &str,
        limit: Option<usize>,
    ) -> PyResult<Vec<PyTrack>> {
        let result = self.runtime.block_on(async {
            let mut iterator = self.client.album_tracks(album_name, artist_name);
            if let Some(limit) = limit {
                iterator.take(limit).await
            } else {
                iterator.collect_all().await
            }
        });

        match result {
            Ok(tracks) => Ok(tracks.into_iter().map(PyTrack::from).collect()),
            Err(e) => Err(PyErr::new::<pyo3::exceptions::PyException, _>(format!(
                "Failed to get album tracks: {}",
                e
            ))),
        }
    }

    /// Get recent tracks (returns up to limit tracks)
    #[pyo3(signature = (limit = None))]
    fn get_recent_tracks(&self, limit: Option<usize>) -> PyResult<Vec<PyTrack>> {
        let result = self.runtime.block_on(async {
            let mut iterator = self.client.recent_tracks();
            if let Some(limit) = limit {
                iterator.take(limit).await
            } else {
                iterator.collect_all().await
            }
        });

        match result {
            Ok(tracks) => Ok(tracks.into_iter().map(PyTrack::from).collect()),
            Err(e) => Err(PyErr::new::<pyo3::exceptions::PyException, _>(format!(
                "Failed to get recent tracks: {}",
                e
            ))),
        }
    }

    /// Get all artists (returns up to limit artists)
    #[pyo3(signature = (limit = None))]
    fn get_artists(&self, limit: Option<usize>) -> PyResult<Vec<PyArtist>> {
        let result = self.runtime.block_on(async {
            let mut iterator = self.client.artists();
            if let Some(limit) = limit {
                iterator.take(limit).await
            } else {
                iterator.collect_all().await
            }
        });

        match result {
            Ok(artists) => Ok(artists.into_iter().map(PyArtist::from).collect()),
            Err(e) => Err(PyErr::new::<pyo3::exceptions::PyException, _>(format!(
                "Failed to get artists: {}",
                e
            ))),
        }
    }

    /// Search for tracks
    #[pyo3(signature = (query, limit = None))]
    fn search_tracks(&self, query: &str, limit: Option<usize>) -> PyResult<Vec<PyTrack>> {
        let result = self.runtime.block_on(async {
            let mut iterator = self.client.search_tracks(query);
            if let Some(limit) = limit {
                iterator.take(limit).await
            } else {
                iterator.collect_all().await
            }
        });

        match result {
            Ok(tracks) => Ok(tracks.into_iter().map(PyTrack::from).collect()),
            Err(e) => Err(PyErr::new::<pyo3::exceptions::PyException, _>(format!(
                "Failed to search tracks: {}",
                e
            ))),
        }
    }

    /// Search for albums
    #[pyo3(signature = (query, limit = None))]
    fn search_albums(&self, query: &str, limit: Option<usize>) -> PyResult<Vec<PyAlbum>> {
        let result = self.runtime.block_on(async {
            let mut iterator = self.client.search_albums(query);
            if let Some(limit) = limit {
                iterator.take(limit).await
            } else {
                iterator.collect_all().await
            }
        });

        match result {
            Ok(albums) => Ok(albums.into_iter().map(PyAlbum::from).collect()),
            Err(e) => Err(PyErr::new::<pyo3::exceptions::PyException, _>(format!(
                "Failed to search albums: {}",
                e
            ))),
        }
    }

    /// Edit a scrobble
    fn edit_scrobble(&self, edit: &PyScrobbleEdit) -> PyResult<PyEditResponse> {
        let result = self
            .runtime
            .block_on(self.client.edit_scrobble(&edit.inner));
        match result {
            Ok(response) => Ok(PyEditResponse { inner: response }),
            Err(e) => Err(PyErr::new::<pyo3::exceptions::PyException, _>(format!(
                "Failed to edit scrobble: {}",
                e
            ))),
        }
    }

    /// Delete a scrobble
    fn delete_scrobble(
        &self,
        artist_name: &str,
        track_name: &str,
        timestamp: u64,
    ) -> PyResult<bool> {
        let result = self.runtime.block_on(self.client.delete_scrobble(
            artist_name,
            track_name,
            timestamp,
        ));
        match result {
            Ok(success) => Ok(success),
            Err(e) => Err(PyErr::new::<pyo3::exceptions::PyException, _>(format!(
                "Failed to delete scrobble: {}",
                e
            ))),
        }
    }

    /// Find a recent scrobble for a specific track
    #[pyo3(signature = (track_name, artist_name, max_pages = 5))]
    fn find_recent_scrobble_for_track(
        &self,
        track_name: &str,
        artist_name: &str,
        max_pages: u32,
    ) -> PyResult<Option<PyTrack>> {
        let result = self
            .runtime
            .block_on(self.client.find_recent_scrobble_for_track(
                track_name,
                artist_name,
                max_pages,
            ));
        match result {
            Ok(track) => Ok(track.map(PyTrack::from)),
            Err(e) => Err(PyErr::new::<pyo3::exceptions::PyException, _>(format!(
                "Failed to find recent scrobble: {}",
                e
            ))),
        }
    }

    /// Get session information
    fn get_session(&self) -> PyLastFmEditSession {
        let session = self.client.get_session();
        PyLastFmEditSession {
            username: session.username,
            base_url: session.base_url,
        }
    }
}

/// A Python module implemented in Rust.
#[pymodule]
fn _lastfm_edit(_py: Python, m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyLastFmEditClient>()?;
    m.add_class::<PyTrack>()?;
    m.add_class::<PyAlbum>()?;
    m.add_class::<PyArtist>()?;
    m.add_class::<PyScrobbleEdit>()?;
    m.add_class::<PyEditResponse>()?;
    m.add_class::<PyLastFmEditSession>()?;
    Ok(())
}
