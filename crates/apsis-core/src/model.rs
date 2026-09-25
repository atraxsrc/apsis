// SPDX-License-Identifier: GPL-3.0-only

use jiff::civil::DateTime;

/// Why a snapshot was taken, as shown in Timeshift's `Tags` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tag {
    /// `O`
    OnDemand,
    /// `B`
    Boot,
    /// `H`
    Hourly,
    /// `D`
    Daily,
    /// `W`
    Weekly,
    /// `M`
    Monthly,
}

impl Tag {
    /// Maps one letter of the `Tags` column to a tag.
    #[must_use]
    pub fn from_char(c: char) -> Option<Self> {
        match c {
            'O' => Some(Self::OnDemand),
            'B' => Some(Self::Boot),
            'H' => Some(Self::Hourly),
            'D' => Some(Self::Daily),
            'W' => Some(Self::Weekly),
            'M' => Some(Self::Monthly),
            _ => None,
        }
    }

    /// The letter Timeshift uses for this tag.
    #[must_use]
    pub fn letter(self) -> char {
        match self {
            Self::OnDemand => 'O',
            Self::Boot => 'B',
            Self::Hourly => 'H',
            Self::Daily => 'D',
            Self::Weekly => 'W',
            Self::Monthly => 'M',
        }
    }

    /// Lower-case English name, as in Timeshift's `--help`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::OnDemand => "on-demand",
            Self::Boot => "boot",
            Self::Hourly => "hourly",
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
        }
    }
}

/// How Timeshift stores snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Btrfs,
    Rsync,
}

/// One snapshot as listed by the backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Timeshift's name, `YYYY-MM-DD_HH-MM-SS`. Used to address the snapshot.
    pub name: String,
    /// Local creation time, taken from the name.
    pub created: DateTime,
    pub tags: Vec<Tag>,
    pub comment: Option<String>,
}

/// The result of listing snapshots on the backup device.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotList {
    /// Backup device path as reported by Timeshift. `None` when no device is selected.
    pub device: Option<String>,
    /// Filesystem UUID of the backup device.
    pub uuid: Option<String>,
    pub mode: Option<Mode>,
    pub snapshots: Vec<Snapshot>,
    /// What Timeshift complained about in an otherwise good list: its `E:`/`W:` lines (e.g.
    /// `E: Failed to remove directory` after a stale mount), and table lines that weren't
    /// snapshot rows. Shown to the user; they don't fail the list.
    pub warnings: Vec<String>,
}

impl SnapshotList {
    /// Value for `--snapshot-device`: the UUID, or the device path when there is no UUID.
    #[must_use]
    pub fn snapshot_device(&self) -> Option<&str> {
        self.uuid.as_deref().or(self.device.as_deref())
    }
}

/// Parses a Timeshift snapshot name (`YYYY-MM-DD_HH-MM-SS`) into its local creation time.
///
/// Anything else, including out-of-range dates, returns `None`.
#[must_use]
pub fn parse_snapshot_name(name: &str) -> Option<DateTime> {
    // YYYY-MM-DD_HH-MM-SS
    // 0123456789012345678
    let bytes = name.as_bytes();
    if bytes.len() != 19 {
        return None;
    }
    let shape_ok = bytes.iter().enumerate().all(|(i, &b)| match i {
        4 | 7 | 13 | 16 => b == b'-',
        10 => b == b'_',
        _ => b.is_ascii_digit(),
    });
    if !shape_ok {
        return None;
    }
    let field = |start: usize, end: usize| name[start..end].parse::<i8>().ok();
    DateTime::new(
        name[0..4].parse().ok()?,
        field(5, 7)?,
        field(8, 10)?,
        field(11, 13)?,
        field(14, 16)?,
        field(17, 19)?,
        0,
    )
    .ok()
}
