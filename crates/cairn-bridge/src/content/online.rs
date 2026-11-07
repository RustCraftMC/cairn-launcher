//! Online resource filtering. Search and install live in `catalog.rs`
//! (`search_online`, `install_online`); both read `m.online.filter` to choose
//! Modrinth or CurseForge, plus the category and sort.

use crate::{Bridge, Model, OnlineFilter};

impl Bridge {
    /// Sets the online filter (source, category, sort) and searches again.
    /// `query` is the search box text. An empty query with no category clears the results,
    /// and an empty query with a category browses that category. The results are also cleared
    /// when the category has no online search. Clearing keeps `m.online.filter`.
    pub fn set_online_filter(&self, m: &mut Model, filter: OnlineFilter, query: &str) {
        m.online.filter = filter;
        self.search_online(m, query);
    }
}
