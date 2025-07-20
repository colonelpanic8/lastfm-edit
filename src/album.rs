use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct Album {
    pub name: String,
    pub artist: String,
    pub playcount: u32,
    pub timestamp: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct AlbumPage {
    pub albums: Vec<Album>,
    pub page_number: u32,
    pub has_next_page: bool,
    pub total_pages: Option<u32>,
}

impl Album {
    pub fn scrobbled_at(&self) -> Option<DateTime<Utc>> {
        self.timestamp
            .and_then(|ts| DateTime::from_timestamp(ts as i64, 0))
    }
}
