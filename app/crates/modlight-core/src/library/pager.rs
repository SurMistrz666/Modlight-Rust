//! Paging the library the way the web panel pages it (`LibraryPager.cs`,
//! `LibraryPagerViewModel.cs`).

use crate::culture::Culture;
use crate::l10n::{Arg, format, s, t};

/// The sizes the panel offers.
pub const PAGE_SIZES: [i64; 3] = [30, 60, 120];

/// The size the owner named, and the server's MAX_PER_PAGE.
pub const DEFAULT_PAGE_SIZE: i64 = 120;

/// Up to this many pages the strip shows every number.
pub const STRIP_LIMIT: i64 = 7;

/// A size no button offers becomes the default.
pub fn clamp_page_size(size: i64) -> i64 {
    if PAGE_SIZES.contains(&size) {
        size
    } else {
        DEFAULT_PAGE_SIZE
    }
}

/// Pages for `total` files; an empty listing is one page, not zero. (The total is the
/// server's number, so the arithmetic here and below cannot be allowed to overflow.)
pub fn page_count(total: i64, per_page: i64) -> i64 {
    if per_page <= 0 {
        return 1;
    }
    1.max(ceil_div(total, per_page))
}

/// `a / b` rounded up, for `b > 0`, without the `a + b - 1` that overflows near the top.
fn ceil_div(a: i64, b: i64) -> i64 {
    a / b + i64::from(a % b > 0)
}

/// The 1-based number of the first file on `page`.
pub fn first(page: i64, per_page: i64) -> i64 {
    (page.max(1) - 1)
        .saturating_mul(per_page.max(1))
        .saturating_add(1)
}

/// The 1-based number of the last file on `page`.
pub fn last(page: i64, per_page: i64, total: i64) -> i64 {
    total
        .max(0)
        .min(page.max(1).saturating_mul(per_page.max(1)))
}

/// The page of `new_size` that holds the file `first_on_screen`.
pub fn anchor(first_on_screen: i64, new_size: i64) -> i64 {
    1.max(ceil_div(first_on_screen.max(1), new_size.max(1)))
}

pub fn clamp_page(page: i64, total_pages: i64) -> i64 {
    page.clamp(1, total_pages.max(1))
}

/// The page buttons, with 0 standing for a gap: `1 . 5 6 7 . 58`.
pub fn strip(page: i64, total_pages: i64) -> Vec<i64> {
    let total_pages = total_pages.max(1);
    let page = clamp_page(page, total_pages);
    if total_pages <= STRIP_LIMIT {
        return (1..=total_pages).collect();
    }
    let mut steps = vec![1];
    let start = 2.max(page - 1);
    let end = (total_pages - 1).min(page + 1);
    if start > 2 {
        steps.push(0);
    }
    steps.extend(start..=end);
    if end < total_pages - 1 {
        steps.push(0);
    }
    steps.push(total_pages);
    steps
}

/// One button of the strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageStep {
    /// The page, or 0 for a gap.
    pub page: i64,
    pub is_current: bool,
    pub label: String,
}

impl PageStep {
    pub fn is_gap(&self) -> bool {
        self.page <= 0
    }
}

/// What the pager bar shows (`LibraryPagerViewModel`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pager {
    pub page: i64,
    pub per_page: i64,
    pub total: i64,
    pub showing_text: String,
    pub page_of_text: String,
    pub jump_text: String,
    pub steps: Vec<PageStep>,
}

impl Default for Pager {
    fn default() -> Self {
        let mut pager = Pager {
            page: 1,
            per_page: DEFAULT_PAGE_SIZE,
            total: 0,
            showing_text: String::new(),
            page_of_text: String::new(),
            jump_text: String::new(),
            steps: Vec::new(),
        };
        pager.update(1, DEFAULT_PAGE_SIZE, 0);
        pager
    }
}

impl Pager {
    pub fn update(&mut self, current_page: i64, current_per_page: i64, current_total: i64) {
        self.total = current_total.max(0);
        self.per_page = clamp_page_size(if current_per_page > 0 {
            current_per_page
        } else {
            self.per_page
        });
        self.page = clamp_page(current_page, page_count(self.total, self.per_page));
        self.refresh_texts();
        self.jump_text.clear();
    }

    /// Re-render the texts, e.g. after the language changed.
    pub fn refresh_texts(&mut self) {
        let culture = Culture::current();
        let first = first(self.page, self.per_page);
        let last = last(self.page, self.per_page, self.total);
        self.showing_text = if self.total > 0 {
            format(
                t(s::MySpace_PageShowing),
                &[
                    Arg::Text(culture.n0(first)),
                    Arg::Text(culture.n0(last)),
                    Arg::Text(culture.n0(self.total)),
                ],
            )
        } else {
            String::new()
        };
        self.page_of_text = format(
            t(s::MySpace_PageOf),
            &[
                Arg::Text(culture.n0(self.page)),
                Arg::Text(culture.n0(self.page_count())),
            ],
        );
        self.steps = strip(self.page, self.page_count())
            .into_iter()
            .map(|step| PageStep {
                page: step,
                is_current: step == self.page,
                label: if step > 0 {
                    culture.n0(step)
                } else {
                    "\u{2026}".to_string()
                },
            })
            .collect();
    }

    pub fn reset(&mut self) {
        let per_page = self.per_page;
        self.update(1, per_page, 0);
    }

    pub fn is_visible(&self) -> bool {
        self.total > 0
    }

    pub fn has_pages(&self) -> bool {
        self.page_count() > 1
    }

    /// The size picker is pointless when even the smallest size shows everything.
    pub fn show_sizes(&self) -> bool {
        self.total > PAGE_SIZES[0]
    }

    pub fn show_jump(&self) -> bool {
        self.page_count() > STRIP_LIMIT
    }

    pub fn can_previous(&self) -> bool {
        self.page > 1
    }

    pub fn can_next(&self) -> bool {
        self.page < self.page_count()
    }

    pub fn page_count(&self) -> i64 {
        page_count(self.total, self.per_page)
    }

    /// The page to go to for `target`, or `None` when it is the page already shown.
    pub fn go_to(&self, target: i64) -> Option<i64> {
        let clamped = clamp_page(target, self.page_count());
        (clamped != self.page).then_some(clamped)
    }

    /// The page typed into the jump box, if it is a page; the box is emptied either way.
    pub fn jump(&mut self) -> Option<i64> {
        let target = self.jump_text.trim().parse::<i64>().ok().filter(|t| *t > 0);
        self.jump_text.clear();
        target.and_then(|t| self.go_to(t))
    }

    /// "30 per page" and its siblings.
    pub fn size_label(size: i64) -> String {
        format(t(s::MySpace_PerPage), &[Arg::Int(size)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_text(steps: Vec<i64>) -> String {
        steps
            .into_iter()
            .map(|s| if s > 0 { s.to_string() } else { ".".into() })
            .collect::<Vec<_>>()
            .join(" ")
    }

    // ModlightLibraryTests.PageArithmeticMatchesThePanel
    #[test]
    fn page_arithmetic_matches_the_panel() {
        assert_eq!(
            page_count(6926, 120),
            58,
            "6,926 files at 120 a page is 58 pages"
        );
        assert_eq!(
            page_count(6926, 30),
            231,
            "6,926 files at 30 a page is 231 pages"
        );
        assert_eq!(first(6, 120), 601, "page 6 of 120 starts at file 601");
        assert_eq!(last(6, 120, 6926), 720, "page 6 of 120 ends at file 720");
        assert_eq!(
            last(58, 120, 6926),
            6926,
            "the last page ends at the last file, not past it"
        );
        assert_eq!(page_count(0, 120), 1, "an empty listing is one page");
        assert_eq!(
            clamp_page(99, 1),
            1,
            "a page past the end clamps to one that exists"
        );
        assert_eq!(clamp_page(0, 58), 1, "page zero clamps to page one");
    }

    // ModlightLibraryTests.PageSizesAreThePanelsOwn
    #[test]
    fn page_sizes_are_the_panels_own() {
        assert_eq!(
            PAGE_SIZES,
            [30, 60, 120],
            "the sizes offered are the panel's 30 / 60 / 120"
        );
        assert_eq!(
            DEFAULT_PAGE_SIZE, 120,
            "the default is the size the owner named"
        );
        assert_eq!(
            clamp_page_size(500),
            120,
            "a size above the server's ceiling becomes the default"
        );
        assert_eq!(
            clamp_page_size(45),
            120,
            "a size no button offers becomes the default"
        );
        assert_eq!(
            clamp_page_size(0),
            120,
            "a missing size becomes the default"
        );
        assert_eq!(clamp_page_size(30), 30, "a size that is offered is kept");
    }

    // ModlightLibraryTests.TheStripElidesLikeThePanels
    #[test]
    fn the_strip_elides_like_the_panels() {
        assert_eq!(
            strip_text(strip(1, 5)),
            "1 2 3 4 5",
            "five pages are all shown"
        );
        assert_eq!(
            strip_text(strip(1, 7)),
            "1 2 3 4 5 6 7",
            "seven pages are all shown"
        );
        assert_eq!(
            strip_text(strip(1, 58)),
            "1 2 . 58",
            "at the start the strip elides on the right"
        );
        assert_eq!(
            strip_text(strip(6, 58)),
            "1 . 5 6 7 . 58",
            "in the middle it elides on both sides"
        );
        assert_eq!(
            strip_text(strip(58, 58)),
            "1 . 57 58",
            "at the end it elides on the left"
        );
    }

    // ModlightLibraryTests.ChangingTheSizeKeepsYourPlace
    #[test]
    fn changing_the_size_keeps_your_place() {
        let first_file = first(101, 30);
        assert_eq!(first_file, 3001, "page 101 of 30 starts at file 3,001");
        assert_eq!(
            anchor(first_file, 120),
            26,
            "the same file is on page 26 of 120"
        );
        assert_eq!(
            first(26, 120),
            3001,
            "and page 26 of 120 starts on it exactly"
        );
        assert_eq!(anchor(1, 120), 1, "the top of the library stays the top");
    }

    #[test]
    fn the_bar_reports_where_you_are() {
        crate::l10n::set_thread_language(Some(crate::l10n::Language::English));
        crate::culture::set_thread_culture(Some(Culture::english()));
        let mut pager = Pager::default();
        assert!(!pager.is_visible());
        pager.update(6, 120, 6926);
        assert_eq!(
            pager.showing_text,
            format(
                t(s::MySpace_PageShowing),
                &[Arg::from("601"), Arg::from("720"), Arg::from("6,926")]
            )
        );
        assert!(pager.show_jump() && pager.has_pages() && pager.show_sizes());
        assert_eq!(pager.go_to(6), None);
        assert_eq!(pager.go_to(999), Some(58));
        pager.jump_text = " 7 ".into();
        assert_eq!(pager.jump(), Some(7));
        assert!(pager.jump_text.is_empty());
        pager.jump_text = "x".into();
        assert_eq!(pager.jump(), None);
        let labels: Vec<String> = pager.steps.iter().map(|s| s.label.clone()).collect();
        assert_eq!(
            labels,
            vec!["1", "\u{2026}", "5", "6", "7", "\u{2026}", "58"]
        );
        crate::l10n::set_thread_language(None);
        crate::culture::set_thread_culture(None);
    }

    /// `total` is a number from the server: an absurd one must not overflow the paging
    /// arithmetic (a panic in a debug build, a negative page count in a release one).
    #[test]
    fn absurd_totals_do_not_overflow() {
        assert_eq!(page_count(i64::MAX, 120), i64::MAX / 120 + 1);
        let pages = page_count(i64::MAX, 30);
        assert_eq!(last(pages, 30, i64::MAX), i64::MAX);
        assert!(first(pages, 30) > 0);
        assert_eq!(anchor(i64::MAX, 120), i64::MAX / 120 + 1);
        let mut pager = Pager::default();
        pager.update(i64::MAX, 120, i64::MAX);
        assert_eq!(pager.page, i64::MAX / 120 + 1);
        assert!(pager.can_previous() && !pager.can_next());
    }
}
