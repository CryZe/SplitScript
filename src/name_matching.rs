//! Shared source-name similarity helpers.
//!
//! Broad lookup suggestions tolerate a few edits for longer names. Syntax
//! domains with a small closed vocabulary can instead require one clear typo,
//! avoiding misleading suggestions for valid-looking unsupported names.

use std::collections::HashSet;

pub(crate) fn closest_name<'a>(
    name: &str,
    candidates: impl Iterator<Item = &'a str>,
) -> Option<String> {
    let normalized_name = normalize_name(name);
    let maximum_distance = match normalized_name.chars().count() {
        0..=3 => 1,
        4..=7 => 2,
        _ => 3,
    };
    closest_with(name, candidates, |candidate| {
        let distance = edit_distance(&normalized_name, &normalize_name(candidate));
        (distance <= maximum_distance).then_some(distance)
    })
}

pub(crate) fn closest_single_typo<'a>(
    name: &str,
    candidates: impl Iterator<Item = &'a str>,
) -> Option<String> {
    let normalized_name = normalize_name(name);
    closest_with(name, candidates, |candidate| {
        let candidate = normalize_name(candidate);
        let distance = edit_distance(&normalized_name, &candidate);
        (distance <= 1 || is_adjacent_transposition(&normalized_name, &candidate))
            .then_some(distance.min(1))
    })
}

fn closest_with<'a>(
    name: &str,
    candidates: impl Iterator<Item = &'a str>,
    mut distance: impl FnMut(&str) -> Option<usize>,
) -> Option<String> {
    let mut seen = HashSet::new();
    let mut best: Option<(usize, String)> = None;
    let mut tied = false;
    for candidate in candidates {
        if candidate == name || !seen.insert(candidate) {
            continue;
        }
        let Some(distance) = distance(candidate) else {
            continue;
        };
        match &best {
            None => {
                best = Some((distance, candidate.to_owned()));
                tied = false;
            }
            Some((best_distance, _)) if distance < *best_distance => {
                best = Some((distance, candidate.to_owned()));
                tied = false;
            }
            Some((best_distance, _)) if distance == *best_distance => tied = true,
            Some(_) => {}
        }
    }
    (!tied)
        .then(|| best.map(|(_, candidate)| candidate))
        .flatten()
}

fn normalize_name(name: &str) -> String {
    name.chars()
        .filter(|character| *character != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];
    for (left_index, left_character) in left.chars().enumerate() {
        current[0] = left_index + 1;
        for (right_index, right_character) in right.iter().enumerate() {
            current[right_index + 1] = (previous[right_index + 1] + 1)
                .min(current[right_index] + 1)
                .min(previous[right_index] + usize::from(left_character != *right_character));
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

fn is_adjacent_transposition(left: &str, right: &str) -> bool {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    if left.len() != right.len() {
        return false;
    }
    let differences = left
        .iter()
        .zip(&right)
        .enumerate()
        .filter_map(|(index, (left, right))| (left != right).then_some(index))
        .collect::<Vec<_>>();
    matches!(differences.as_slice(), [first, second]
        if *second == *first + 1
            && left[*first] == right[*second]
            && left[*second] == right[*first])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_typo_matching_accepts_case_edits_and_transpositions() {
        let candidates = ["Native", "Unity", "GBA"];
        assert_eq!(
            closest_single_typo("unity", candidates.into_iter()),
            Some("Unity".to_owned())
        );
        assert_eq!(
            closest_single_typo("Untiy", candidates.into_iter()),
            Some("Unity".to_owned())
        );
    }

    #[test]
    fn single_typo_matching_rejects_merely_nearby_unsupported_names() {
        let candidates = ["SMS", "Genesis", "GBA"];
        assert_eq!(closest_single_typo("SNES", candidates.into_iter()), None);
    }
}
