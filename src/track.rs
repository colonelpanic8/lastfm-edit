#[derive(Debug, Clone)]
pub struct Track {
    pub name: String,
    pub artist: String,
    pub playcount: u32,
}

#[derive(Debug, Clone)]
pub struct TrackPage {
    pub tracks: Vec<Track>,
    pub page_number: u32,
    pub has_next_page: bool,
    pub total_pages: Option<u32>,
}
