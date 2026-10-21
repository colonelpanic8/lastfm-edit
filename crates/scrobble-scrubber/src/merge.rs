//! Combining several correctors' edits of one subject into a single edit, field by field.
//!
//! Correctors are ranked by their position in the configured order (0 is the highest
//! priority). For each field group, the best-ranked proposal that changes it supplies the
//! whole group.

use lastfm_edit::ScrobbleEdit;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditField {
    Track,
    Artist,
    Album,
    AlbumArtist,
}

impl EditField {
    pub const ALL: [Self; 4] = [Self::Track, Self::Artist, Self::Album, Self::AlbumArtist];

    /// Fields one corrector supplies together. An album artist is the credit of its album,
    /// so it only makes sense with the album it came with.
    const GROUPS: [&'static [Self]; 3] = [
        &[Self::Track],
        &[Self::Artist],
        &[Self::Album, Self::AlbumArtist],
    ];

    /// The value `edit` writes to this field, when that differs from the original. This is
    /// what the executor applies.
    pub fn change(self, edit: &ScrobbleEdit) -> Option<&str> {
        let (original, new) = match self {
            Self::Track => (
                edit.track_name_original.as_deref(),
                edit.track_name.as_deref(),
            ),
            Self::Artist => (
                Some(edit.artist_name_original.as_str()),
                Some(edit.artist_name.as_str()),
            ),
            Self::Album => (
                edit.album_name_original.as_deref(),
                edit.album_name.as_deref(),
            ),
            Self::AlbumArtist => (
                edit.album_artist_name_original.as_deref(),
                edit.album_artist_name.as_deref(),
            ),
        };
        new.filter(|new| original != Some(*new))
    }

    /// Write `value` to this field, or restore the original when `None`.
    fn set(self, edit: &mut ScrobbleEdit, value: Option<&str>) {
        let value = value.map(str::to_string);
        match self {
            Self::Track => edit.track_name = value.or_else(|| edit.track_name_original.clone()),
            Self::Artist => {
                edit.artist_name = value.unwrap_or_else(|| edit.artist_name_original.clone())
            }
            Self::Album => edit.album_name = value.or_else(|| edit.album_name_original.clone()),
            Self::AlbumArtist => {
                edit.album_artist_name = value.or_else(|| edit.album_artist_name_original.clone())
            }
        }
    }
}

/// Every field `edit` changes, with its new value.
pub fn changes(edit: &ScrobbleEdit) -> BTreeMap<EditField, &str> {
    EditField::ALL
        .into_iter()
        .filter_map(|field| field.change(edit).map(|value| (field, value)))
        .collect()
}

/// `edit` with only `fields` changed; every other field keeps its original.
pub fn restrict(edit: &ScrobbleEdit, fields: &BTreeSet<EditField>) -> ScrobbleEdit {
    let mut restricted = edit.clone();
    for field in EditField::ALL {
        if !fields.contains(&field) {
            field.set(&mut restricted, None);
        }
    }
    restricted
}

/// One corrector's edit of a subject.
#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    pub provider: String,
    pub rank: usize,
    pub edit: ScrobbleEdit,
    /// Already capped by the corrector's ceiling.
    pub requires_confirmation: bool,
    pub rationale: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Merged {
    pub edit: ScrobbleEdit,
    /// The best-ranked contributor.
    pub provider: String,
    /// Who set each changed field.
    pub sources: BTreeMap<EditField, String>,
    pub requires_confirmation: bool,
    /// Contributors' rationales, best rank first.
    pub rationale: Option<String>,
}

/// Merge `proposals` group by group (see [`EditField::GROUPS`]). A group's winner supplies
/// all of its fields, keeping the original where it leaves one alone. `owned` holds, per
/// field, the rank of the corrector that last set it on this subject; a worse-ranked
/// proposal may not change that field's group. Equal ranks resolve to the earlier
/// proposal. `None` when nothing changes.
pub fn merge(proposals: &[Proposal], owned: &BTreeMap<EditField, usize>) -> Option<Merged> {
    let winners: BTreeMap<EditField, usize> = EditField::GROUPS
        .into_iter()
        .filter_map(|group| {
            let owner = group.iter().filter_map(|field| owned.get(field)).min();
            let (index, winner) = proposals
                .iter()
                .enumerate()
                .filter(|(_, proposal)| {
                    group
                        .iter()
                        .any(|field| field.change(&proposal.edit).is_some())
                })
                .filter(|(_, proposal)| owner.is_none_or(|&owner| proposal.rank <= owner))
                .min_by_key(|(_, proposal)| proposal.rank)?;
            Some(
                group
                    .iter()
                    .filter(|field| field.change(&winner.edit).is_some())
                    .map(move |&field| (field, index)),
            )
        })
        .flatten()
        .collect();

    let mut contributors: Vec<&Proposal> = winners
        .values()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|&index| &proposals[index])
        .collect();
    contributors.sort_by_key(|proposal| proposal.rank);
    let lead = contributors.first()?;

    let mut edit = lead.edit.clone();
    for field in EditField::ALL {
        let value = winners
            .get(&field)
            .and_then(|&index| field.change(&proposals[index].edit));
        field.set(&mut edit, value);
    }
    let rationales: Vec<&str> = contributors
        .iter()
        .filter_map(|proposal| proposal.rationale.as_deref())
        .collect();
    Some(Merged {
        edit,
        provider: lead.provider.clone(),
        sources: winners
            .iter()
            .map(|(&field, &index)| (field, proposals[index].provider.clone()))
            .collect(),
        requires_confirmation: contributors
            .iter()
            .any(|proposal| proposal.requires_confirmation),
        rationale: (!rationales.is_empty()).then(|| rationales.join("\n")),
    })
}

#[cfg(test)]
#[path = "merge_tests.rs"]
mod tests;
