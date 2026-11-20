//! Mapping a raw mRemoteNG document to an import plan: a folder subtree, the vault entries it
//! references and the report. Pure and synchronous; nothing is written anywhere.
//!
//! Inheritance is mapped faithfully: an `Inherit*` flag decides local or unset for every setting,
//! folders included. The "inherits now" report lines are computed while descending (the nearest
//! folder above that sets a credential or username), which gives the same answer as
//! `Tree::resolve_node` without searching the tree once per node (a test cross-checks both).

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, RandomState};
use std::rc::Rc;

use fernjo_core::{CredentialId, Node, NodeId, NodeKind, Protocol, RdpResolution, Settings};
use fernjo_vault::{EntryId, NewEntry, Secret};
use secrecy::{ExposeSecret, SecretString};

use super::file::{
    MAX_DESCRIPTION_CHARS, MAX_HOST_CHARS, MAX_IDENTITY_CHARS, MAX_NAME_CHARS, RawDocument,
    RawKind, RawNode,
};
use crate::report::{Counts, MAX_LINE_CHARS, Report, ReportLine, cap};

const UNNAMED: &str = "Unnamed";
const DEFAULT_PUTTY_SESSION: &str = "Default Settings";
const FOLDER_PREFIX: &str = "Imported from mRemoteNG";

/// A vault entry the import wants to create, under the id the tree already references.
#[derive(Debug, Clone)]
pub struct PlannedEntry {
    pub id: EntryId,
    pub entry: NewEntry,
}

/// Everything an import adds: one top-level folder, the entries its nodes reference, the report.
#[derive(Debug, Clone)]
pub struct ImportPlan {
    pub folder: Node,
    pub entries: Vec<PlannedEntry>,
    pub report: Report,
    /// The report of "import without passwords" (the entries and the credential references
    /// are dropped): no credentials, no "now uses the credential of folder X" lines, and one line
    /// saying how many stored password references were left out.
    pub report_without_passwords: Report,
}

/// The nearest folders above a node that set a credential / a username (their names as shown in
/// report lines, see `display`). Shared, so a long name is not cloned for every child folder.
#[derive(Clone, Default)]
struct Above {
    credential: Option<Rc<str>>,
    username: Option<Rc<str>>,
}

/// How a report line reads when the import runs without passwords.
#[derive(Clone)]
enum WithoutPasswords {
    Same,
    /// The line is not true without credentials (a node inheriting a folder's credential).
    Drop,
    /// The line reads differently.
    Text(String),
}

struct Line {
    line: ReportLine,
    without_passwords: WithoutPasswords,
}

/// (entry username, entry domain, keyed hash of the password).
type EntryKey = (Option<String>, Option<String>, u64);

#[derive(Default)]
struct Builder {
    entries: Vec<PlannedEntry>,
    uses: Vec<usize>,
    /// Key -> indexes into `entries`. The hash only narrows the lookup (random per-process keys,
    /// never stored or printed); a hit is confirmed by comparing the real passwords, so a
    /// collision cannot merge two different passwords.
    by_identity: HashMap<EntryKey, Vec<usize>>,
    hasher: RandomState,
    entry_names: HashSet<String>,
    /// Base entry name -> the next ` (n)` suffix to try, so numbering never restarts at 2.
    next_suffix: HashMap<String, usize>,
    counts: Counts,
    /// Nodes that reference an entry (what "import without passwords" leaves out).
    references: usize,
    skipped: Vec<ReportLine>,
    partial: Vec<Line>,
    problems: Vec<Line>,
}

/// Builds the plan. `today` is `YYYY-MM-DD`; `taken_names` are the existing top-level names, used
/// for the ` (2)`, ` (3)` suffix of the new folder.
pub fn plan(doc: &RawDocument, today: &str, taken_names: &[String]) -> ImportPlan {
    let mut b = Builder::default();
    let above = Above::default();
    let mut trail = Vec::new();
    let children: Vec<Node> = doc
        .nodes
        .iter()
        .filter_map(|n| b.convert(n, &mut trail, &above))
        .collect();

    let base = format!("{FOLDER_PREFIX} ({today})");
    let name = crate::naming::unique_name(&base, taken_names);
    let folder = Node {
        id: NodeId::new_v4(),
        name,
        description: String::new(),
        settings: Settings::default(),
        kind: NodeKind::Folder { children },
    };
    let (report, report_without_passwords) = b.finish();
    ImportPlan {
        folder,
        entries: b.entries,
        report,
        report_without_passwords,
    }
}

/// The part of `s` a report line can show: its first `MAX_LINE_CHARS` characters. A line is cut to
/// that many anyway, so anything copied beyond it is wasted, and with an attacker-chosen name
/// times the number of nodes it adds up to minutes. Names pass through here wherever they are
/// put into a trail or a message; the cut text reads the same as before in the line.
/// mRemoteNG `Resolution` to a setting: the window and full screen modes follow the tab, `ResWxH`
/// is a fixed size (when it is one the core accepts); anything else (`SmartSize`, ...) has no
/// equivalent.
fn map_resolution(text: &str) -> Option<RdpResolution> {
    if text.eq_ignore_ascii_case("FitToWindow") || text.eq_ignore_ascii_case("Fullscreen") {
        return Some(RdpResolution::Fit);
    }
    let size = text.strip_prefix("Res")?;
    let (width, height) = size.split_once('x')?;
    let resolution = RdpResolution::Fixed {
        width: width.parse().ok()?,
        height: height.parse().ok()?,
    };
    resolution.is_valid().then_some(resolution)
}

/// mRemoteNG `Colors` to a colour depth (256 and 15-bit colours have no equivalent).
fn map_colors(text: &str) -> Option<u8> {
    match text {
        "Colors16Bit" => Some(16),
        "Colors24Bit" => Some(24),
        "Colors32Bit" => Some(32),
        _ => None,
    }
}

fn display(s: &str) -> &str {
    match s.char_indices().nth(MAX_LINE_CHARS) {
        None => s,
        Some((end, _)) => &s[..end],
    }
}

/// The report path of the node at the end of `trail` (its components come from `display`). Built
/// only when a line needs it, and not further than the line can show.
fn path(trail: &[&str]) -> String {
    // bytes >= characters, so this many bytes always hold more characters than the line shows
    const ENOUGH_BYTES: usize = 4 * (MAX_LINE_CHARS + 1);
    let mut out = String::new();
    for (i, part) in trail.iter().enumerate() {
        if i > 0 {
            out.push_str(" / ");
        }
        out.push_str(part);
        if out.len() >= ENOUGH_BYTES {
            break;
        }
    }
    out
}

impl Builder {
    /// The report and the report of an import without passwords.
    fn finish(&mut self) -> (Report, Report) {
        let skipped = std::mem::take(&mut self.skipped);
        let partial = std::mem::take(&mut self.partial);
        let problems = std::mem::take(&mut self.problems);

        // Aggregate lines go first so the cap never drops them.
        let mut head = Vec::new();
        if self.counts.rdp > 0 {
            let n = self.counts.rdp;
            let s = if n == 1 { "" } else { "s" };
            head.push(ReportLine::new(
                "",
                format!(
                    "RDP redirection, gateway and the display settings other than resolution and colour depth are not carried over ({n} RDP connection{s})"
                ),
            ));
        }

        let mut normal_partial = head.clone();
        normal_partial.extend(partial.iter().map(|l| l.line.clone()));
        let mut bare_partial = head;
        if self.references > 0 {
            let n = self.references;
            let s = if n == 1 { "" } else { "s" };
            bare_partial.push(ReportLine::new(
                "",
                format!("{n} stored password reference{s} left out (imported without passwords)"),
            ));
        }
        bare_partial.extend(partial.iter().filter_map(|l| match &l.without_passwords {
            WithoutPasswords::Same => Some(l.line.clone()),
            WithoutPasswords::Drop => None,
            WithoutPasswords::Text(text) => Some(ReportLine::new(l.line.path.clone(), text)),
        }));
        let normal_problems: Vec<ReportLine> = problems.iter().map(|l| l.line.clone()).collect();
        let bare_problems: Vec<ReportLine> = problems
            .iter()
            .map(|l| match &l.without_passwords {
                WithoutPasswords::Text(text) => ReportLine::new(l.line.path.clone(), text),
                _ => l.line.clone(),
            })
            .collect();

        let mut counts = self.counts.clone();
        counts.credentials = self.entries.len();
        counts.skipped = skipped.len();
        counts.partial = normal_partial.len();
        counts.problems = normal_problems.len();
        let mut bare_counts = counts.clone();
        bare_counts.credentials = 0;
        bare_counts.shared_credentials = 0;
        bare_counts.partial = bare_partial.len();
        bare_counts.problems = bare_problems.len();
        (
            Report {
                counts,
                skipped: cap(skipped.clone()),
                partial: cap(normal_partial),
                problems: cap(normal_problems),
            },
            Report {
                counts: bare_counts,
                skipped: cap(skipped),
                partial: cap(bare_partial),
                problems: cap(bare_problems),
            },
        )
    }

    fn skip(&mut self, trail: &[&str], text: String) {
        self.skipped.push(ReportLine::new(path(trail), text));
    }
    fn partly(&mut self, trail: &[&str], text: String) {
        self.partial.push(Line {
            line: ReportLine::new(path(trail), text),
            without_passwords: WithoutPasswords::Same,
        });
    }
    fn problem(&mut self, trail: &[&str], text: String) {
        self.problems.push(Line {
            line: ReportLine::new(path(trail), text),
            without_passwords: WithoutPasswords::Same,
        });
    }

    /// Maps one raw node (and below). `None` when the node is skipped. `trail` holds the display
    /// names from the top down to and including this node while it is being mapped.
    fn convert<'a>(
        &mut self,
        raw: &'a RawNode,
        trail: &mut Vec<&'a str>,
        above: &Above,
    ) -> Option<Node> {
        let trimmed = raw.name.trim();
        let name = if trimmed.is_empty() { UNNAMED } else { trimmed };
        trail.push(display(name));
        let node = self.convert_in(raw, name, trimmed.is_empty(), trail, above);
        trail.pop();
        node
    }

    fn convert_in<'a>(
        &mut self,
        raw: &'a RawNode,
        name: &'a str,
        unnamed: bool,
        trail: &mut Vec<&'a str>,
        above: &Above,
    ) -> Option<Node> {
        let protocol = match &raw.kind {
            RawKind::Unknown(t) => {
                self.skip(
                    trail,
                    format!(
                        "node type \"{t}\" is not supported; it and everything below it were skipped"
                    ),
                );
                return None;
            }
            RawKind::Container => None,
            RawKind::Connection => {
                let p = raw.protocol.trim();
                // a cut value is never a protocol, even when what is left reads like one
                if raw.clipped.protocol {
                    self.skip(trail, format!("protocol {}… is not supported", display(p)));
                    return None;
                } else if p.eq_ignore_ascii_case("SSH2") {
                    Some(Protocol::Ssh)
                } else if p.eq_ignore_ascii_case("RDP") {
                    Some(Protocol::Rdp)
                } else {
                    let shown = if p.is_empty() { "(none)" } else { display(p) };
                    self.skip(trail, format!("protocol {shown} is not supported"));
                    return None;
                }
            }
        };

        if unnamed {
            self.problem(trail, "has no name; imported as \"Unnamed\"".to_owned());
        }
        self.clipped_lines(raw, trail);
        let (mut settings, credential_set, username_set) = self.settings(raw, name, trail, above);
        if protocol == Some(Protocol::Rdp) {
            self.rdp_display(raw, trail, &mut settings);
        }

        let kind = match protocol {
            Some(protocol) => {
                let host = raw.hostname.trim().to_owned();
                if host.is_empty() && raw.clipped.hostname {
                    self.problem(
                        trail,
                        format!(
                            "its host is longer than {MAX_HOST_CHARS} characters; imported with an empty host"
                        ),
                    );
                } else if host.is_empty() {
                    self.problem(trail, "has no host; imported with an empty host".to_owned());
                }
                match protocol {
                    Protocol::Ssh => self.counts.ssh += 1,
                    Protocol::Rdp => self.counts.rdp += 1,
                }
                let putty = raw.putty_session.trim();
                if !putty.is_empty() && putty != DEFAULT_PUTTY_SESSION {
                    self.partly(
                        trail,
                        format!("PuTTY session \"{}\" is not carried over", display(putty)),
                    );
                }
                NodeKind::Connection { protocol, host }
            }
            None => {
                self.counts.folders += 1;
                let below = Above {
                    credential: if credential_set {
                        Some(Rc::from(display(name)))
                    } else {
                        above.credential.clone()
                    },
                    username: if username_set {
                        Some(Rc::from(display(name)))
                    } else {
                        above.username.clone()
                    },
                };
                let children = raw
                    .children
                    .iter()
                    .filter_map(|c| self.convert(c, trail, &below))
                    .collect();
                NodeKind::Folder { children }
            }
        };

        Some(Node {
            id: NodeId::new_v4(),
            name: name.to_owned(),
            description: raw.description.clone(),
            settings,
            kind,
        })
    }

    /// One line per attribute of `raw` that was cut at its cap (see `file::MAX_*_CHARS`). A cut host
    /// is reported where the host is mapped.
    fn clipped_lines(&mut self, raw: &RawNode, trail: &[&str]) {
        let c = raw.clipped;
        for (cut, what, max) in [
            (c.name, "name", MAX_NAME_CHARS),
            (c.description, "description", MAX_DESCRIPTION_CHARS),
            (c.username, "username", MAX_IDENTITY_CHARS),
            (c.domain, "domain", MAX_IDENTITY_CHARS),
        ] {
            if cut {
                self.partly(
                    trail,
                    format!(
                        "its {what} was longer than {max} characters and was shortened to {max}"
                    ),
                );
            }
        }
    }

    /// Resolution and colour depth of an RDP connection, unless they are inherited. A value that has
    /// no equivalent here is left unset and listed as partly mapped; an empty one (an old file) is
    /// simply unset.
    fn rdp_display(&mut self, raw: &RawNode, trail: &[&str], s: &mut Settings) {
        let resolution = raw.resolution.trim();
        if !raw.inherit.resolution && !resolution.is_empty() {
            match map_resolution(resolution) {
                Some(r) => s.rdp_resolution = Some(r),
                None => self.partly(
                    trail,
                    format!(
                        "its resolution \"{}\" has no equivalent and is not carried over",
                        display(resolution)
                    ),
                ),
            }
        }
        let colors = raw.colors.trim();
        if !raw.inherit.colors && !colors.is_empty() {
            match map_colors(colors) {
                Some(depth) => s.rdp_color_depth = Some(depth),
                None => self.partly(
                    trail,
                    format!(
                        "its colour depth \"{}\" has no equivalent and is not carried over",
                        display(colors)
                    ),
                ),
            }
        }
    }

    /// The node's own settings. Also returns whether it sets a credential and a username (so a
    /// folder can tell its children where they inherit from).
    fn settings(
        &mut self,
        raw: &RawNode,
        name: &str,
        trail: &[&str],
        above: &Above,
    ) -> (Settings, bool, bool) {
        let mut s = Settings::default();

        if !raw.inherit.port {
            if let Some(p) = raw.port {
                match u16::try_from(p).ok().filter(|p| *p != 0) {
                    Some(p) => s.port = Some(p),
                    None => self.problem(
                        trail,
                        format!("port {p} is out of range; the port is left unset"),
                    ),
                }
            } else if let Some(text) = &raw.port_invalid {
                self.problem(
                    trail,
                    format!("port \"{text}\" is not a number; the port is left unset"),
                );
            }
        }

        let own_username =
            (!raw.inherit.username && !raw.username.is_empty()).then(|| raw.username.clone());
        let own_domain =
            (!raw.inherit.domain && !raw.domain.is_empty()).then(|| raw.domain.clone());
        s.username.clone_from(&own_username);

        if !raw.inherit.password {
            match &raw.password {
                Some(password) => {
                    s.credential =
                        Some(self.entry_for(own_username, own_domain.clone(), password, name));
                }
                None => {
                    if raw.password_error {
                        const PLAIN: &str =
                            "its password could not be decrypted; imported without one";
                        match &above.credential {
                            // the node inherits the folder's credential, so "without one" would
                            // be untrue (unless nothing is imported with a password at all)
                            Some(folder) => self.problems.push(Line {
                                line: ReportLine::new(
                                    path(trail),
                                    format!(
                                        "its password could not be decrypted; it now uses the credential of folder \"{folder}\""
                                    ),
                                ),
                                without_passwords: WithoutPasswords::Text(PLAIN.to_owned()),
                            }),
                            None => self.problem(trail, PLAIN.to_owned()),
                        }
                    } else if let Some(folder) = &above.credential {
                        self.partial.push(Line {
                            line: ReportLine::new(
                                path(trail),
                                format!(
                                    "has no password of its own in mRemoteNG; it now uses the credential of folder \"{folder}\""
                                ),
                            ),
                            without_passwords: WithoutPasswords::Drop,
                        });
                    }
                }
            }
        }
        if let Some(domain) = own_domain
            && s.credential.is_none()
        {
            self.partly(
                trail,
                format!(
                    "domain \"{}\" is not imported (no password to attach it to)",
                    display(&domain)
                ),
            );
        }
        if !raw.inherit.username
            && raw.username.is_empty()
            && let Some(folder) = &above.username
        {
            self.partly(
                trail,
                format!(
                    "has no username of its own in mRemoteNG; it now uses the username of folder \"{folder}\""
                ),
            );
        }

        let credential_set = s.credential.is_some();
        let username_set = s.username.is_some();
        (s, credential_set, username_set)
    }

    /// The id of the entry for this (username, domain, password), creating it when it is new.
    /// `node_name` names an entry that has no username.
    fn entry_for(
        &mut self,
        username: Option<String>,
        domain: Option<String>,
        password: &str,
        node_name: &str,
    ) -> CredentialId {
        self.references += 1;
        let key = (
            username.clone(),
            domain.clone(),
            self.hasher.hash_one(password),
        );
        if let Some(indexes) = self.by_identity.get(&key) {
            for &i in indexes {
                if let Secret::Password { password: stored } = &self.entries[i].entry.secret
                    && stored.expose_secret() == password
                {
                    self.uses[i] += 1;
                    if self.uses[i] == 2 {
                        self.counts.shared_credentials += 1;
                    }
                    return self.entries[i].id;
                }
            }
        }

        let base = match (&username, &domain) {
            (Some(u), Some(d)) => format!("{u}@{d}"),
            (Some(u), None) => u.clone(),
            // No username: named after the first node that uses the entry.
            _ => node_name.to_owned(),
        };
        let mut name = base.clone();
        if self.entry_names.contains(&name) {
            let next = self.next_suffix.entry(base.clone()).or_insert(2);
            loop {
                name = format!("{base} ({next})");
                *next += 1;
                if !self.entry_names.contains(&name) {
                    break;
                }
            }
        }
        self.entry_names.insert(name.clone());

        let id = EntryId::new_v4();
        self.by_identity
            .entry(key)
            .or_default()
            .push(self.entries.len());
        self.entries.push(PlannedEntry {
            id,
            entry: NewEntry {
                name,
                username,
                domain,
                secret: Secret::Password {
                    password: SecretString::from(password.to_owned()),
                },
                notes: String::new(),
            },
        });
        self.uses.push(1);
        id
    }
}
