//! The pages of the site, in one table: the navigation, each page's title and
//! its description come from here, and a page is found by its handler, never
//! by its path.

use topcoat::{
    context::Cx,
    router::{HrefTarget, href},
};

use crate::app::{self, install, recovery, security};

/// A group of documentation pages in the side list.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Start,
    Guides,
    Reference,
}

impl Section {
    pub const ALL: [Self; 3] = [Self::Start, Self::Guides, Self::Reference];

    pub const fn title(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Guides => "Guides",
            Self::Reference => "Reference",
        }
    }
}

pub struct Page {
    pub target: &'static (dyn HrefTarget + Sync),
    /// The name in the side list.
    pub label: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// The group of the side list, for a documentation page.
    pub section: Option<Section>,
}

pub static PAGES: [Page; 4] = [
    Page {
        target: &app::page,
        label: "Home",
        title: "Bufonidae: one folder over several Mega accounts",
        description: "A Linux app and command line that spreads files across several Mega accounts, with optional copies, using rclone.",
        section: None,
    },
    Page {
        target: &install::page,
        label: "Install and use",
        title: "Install and use Bufonidae",
        description: "Requirements, building, connecting accounts and the commands.",
        section: Some(Section::Start),
    },
    Page {
        target: &recovery::page,
        label: "Backup and recovery",
        title: "Backup and recovery - Bufonidae",
        description: "Export a backup, restore it, and what to do with a damaged index.",
        section: Some(Section::Guides),
    },
    Page {
        target: &security::page,
        label: "Security and limits",
        title: "Security and limits - Bufonidae",
        description: "What Bufonidae protects, and what it does not.",
        section: Some(Section::Reference),
    },
];

/// What a request for a page that does not exist is answered with.
pub const NOT_FOUND: (&str, &str) = ("Page not found - Bufonidae", "This page does not exist.");

/// The page this request is for, if it is one of ours.
pub fn current(cx: &Cx) -> Option<&'static Page> {
    PAGES.iter().find(|page| href!(page.target).is_current(cx))
}

/// The documentation pages of a group, in order.
pub fn pages_in(section: Section) -> impl Iterator<Item = &'static Page> {
    PAGES
        .iter()
        .filter(move |page| page.section == Some(section))
}

/// The documentation page after the one this request is for, in the order of
/// the side list. The last page has none.
pub fn next(cx: &Cx) -> Option<&'static Page> {
    let here = current(cx)?;
    let mut order = Section::ALL.into_iter().flat_map(pages_in);
    order.find(|page| std::ptr::eq(*page, here))?;
    order.next()
}
