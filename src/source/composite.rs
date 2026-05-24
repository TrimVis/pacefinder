//! Composite data source: tries each underlying adapter in order.
//!
//! For every method, the first underlying source that returns `Some(...)`
//! wins. This lets you put a fresh-but-incomplete source first (e.g.
//! onepace.net for season descriptions) and a comprehensive-but-stale
//! source second (e.g. SpykerNZ for episode-level data and posters).

use std::rc::Rc;

use anyhow::Result;
use tracing::warn;

use super::{DataSource, ImageKind};
use crate::model::{Episode, Season, Series};

pub struct Composite {
    sources: Vec<Rc<dyn DataSource>>,
}

impl Composite {
    pub fn new(sources: Vec<Rc<dyn DataSource>>) -> Self {
        Self { sources }
    }

    /// Try each source in order; return the first `Some`, log and skip on
    /// `Err`, fall through on `Ok(None)`. `op` is included in the warning
    /// so a failure pinpoints which method drifted.
    fn try_each<T>(
        &self,
        op: &'static str,
        mut f: impl FnMut(&dyn DataSource) -> Result<Option<T>>,
    ) -> Result<Option<T>> {
        for s in &self.sources {
            match f(s.as_ref()) {
                Ok(Some(v)) => return Ok(Some(v)),
                Ok(None) => continue,
                Err(e) => warn!(source = s.name(), op, error = %e, "fetch failed"),
            }
        }
        Ok(None)
    }
}

impl DataSource for Composite {
    fn name(&self) -> &'static str {
        "composite"
    }

    fn series(&self) -> Result<Option<Series>> {
        self.try_each("series", |s| s.series())
    }

    fn season(&self, number: u32) -> Result<Option<Season>> {
        self.try_each("season", |s| s.season(number))
    }

    fn episode(&self, arc_normalized: &str, episode_number: u32) -> Result<Option<Episode>> {
        self.try_each("episode", |s| s.episode(arc_normalized, episode_number))
    }

    fn image(&self, kind: ImageKind) -> Result<Option<Vec<u8>>> {
        self.try_each("image", |s| s.image(kind))
    }

    fn identify_by_crc(&self, crc: &str) -> Result<Option<(String, u32)>> {
        self.try_each("identify_by_crc", |s| s.identify_by_crc(crc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{NamedSeason, Series};
    use anyhow::anyhow;
    use std::cell::RefCell;

    /// Stub adapter: records every call and returns canned outputs.
    struct Stub {
        name: &'static str,
        series: Option<Series>,
        season: Option<Season>,
        episode: Option<Episode>,
        image: Option<Vec<u8>>,
        series_err: bool,
        calls: RefCell<Vec<&'static str>>,
    }

    impl Stub {
        fn empty(name: &'static str) -> Self {
            Self {
                name,
                series: None,
                season: None,
                episode: None,
                image: None,
                series_err: false,
                calls: RefCell::new(Vec::new()),
            }
        }
        fn with_series(mut self, s: Series) -> Self {
            self.series = Some(s);
            self
        }
        fn failing_series(mut self) -> Self {
            self.series_err = true;
            self
        }
    }

    impl DataSource for Stub {
        fn name(&self) -> &'static str {
            self.name
        }
        fn series(&self) -> Result<Option<Series>> {
            self.calls.borrow_mut().push("series");
            if self.series_err {
                Err(anyhow!("intentional"))
            } else {
                Ok(self.series.clone())
            }
        }
        fn season(&self, _: u32) -> Result<Option<Season>> {
            self.calls.borrow_mut().push("season");
            Ok(self.season.clone())
        }
        fn episode(&self, _: &str, _: u32) -> Result<Option<Episode>> {
            self.calls.borrow_mut().push("episode");
            Ok(self.episode.clone())
        }
        fn image(&self, _: ImageKind) -> Result<Option<Vec<u8>>> {
            self.calls.borrow_mut().push("image");
            Ok(self.image.clone())
        }
    }

    fn sample_series(title: &str) -> Series {
        Series {
            title: title.into(),
            showtitle: title.into(),
            original_title: None,
            plot: "x".into(),
            display_order: None,
            named_seasons: vec![NamedSeason {
                number: 1,
                name: "1. x".into(),
            }],
        }
    }

    #[test]
    fn first_some_wins_and_short_circuits() {
        let first = Rc::new(Stub::empty("first").with_series(sample_series("first wins")));
        let second = Rc::new(Stub::empty("second").with_series(sample_series("second")));
        let composite = Composite::new(vec![first.clone(), second.clone()]);

        let s = composite.series().unwrap().unwrap();
        assert_eq!(s.title, "first wins");
        // second was never asked
        assert_eq!(*first.calls.borrow(), vec!["series"]);
        assert!(second.calls.borrow().is_empty());
    }

    #[test]
    fn none_falls_through_to_next() {
        let first = Rc::new(Stub::empty("empty"));
        let second = Rc::new(Stub::empty("loaded").with_series(sample_series("second")));
        let composite = Composite::new(vec![first.clone(), second.clone()]);

        let s = composite.series().unwrap().unwrap();
        assert_eq!(s.title, "second");
        assert_eq!(*first.calls.borrow(), vec!["series"]);
        assert_eq!(*second.calls.borrow(), vec!["series"]);
    }

    #[test]
    fn err_is_logged_and_loop_continues() {
        let first = Rc::new(Stub::empty("broken").failing_series());
        let second = Rc::new(Stub::empty("ok").with_series(sample_series("from-fallback")));
        let composite = Composite::new(vec![first.clone(), second.clone()]);

        let s = composite.series().unwrap().unwrap();
        assert_eq!(s.title, "from-fallback");
        assert_eq!(*first.calls.borrow(), vec!["series"]);
        assert_eq!(*second.calls.borrow(), vec!["series"]);
    }

    #[test]
    fn all_none_returns_none() {
        let first = Rc::new(Stub::empty("a"));
        let second = Rc::new(Stub::empty("b"));
        let composite = Composite::new(vec![first, second]);
        assert!(composite.series().unwrap().is_none());
        assert!(composite.season(1).unwrap().is_none());
        assert!(composite.episode("x", 1).unwrap().is_none());
        assert!(composite.image(ImageKind::SeriesPoster).unwrap().is_none());
    }
}
