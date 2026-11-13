//! Areas: books grouped by meaning.
//!
//! Spherical k-means, built by bisecting: the area that is most spread
//! out is split in two until there are enough. Then every book goes to
//! its nearest centre for a few rounds, which tidies the borders a greedy
//! split leaves. Each area is split once more into sub-areas. The same
//! seed gives the same areas.

use super::rng::Rng;
use super::{Points, dot};
use crate::services::embedding::vector::normalize;

/// Starts tried for each split; the best split is kept.
const RESTARTS: usize = 4;
/// Rounds of one split, and of the tidying afterwards.
const ROUNDS: usize = 30;
const TIDY_ROUNDS: usize = 10;

#[derive(Debug, Clone)]
pub struct AreaOptions {
    /// How many areas to make, at most.
    pub areas: usize,
    /// No area is smaller.
    pub min_area: usize,
    /// How many sub-areas an area gets, at most.
    pub sub_areas: usize,
    pub min_sub_area: usize,
    pub seed: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Area {
    /// Row numbers, ascending.
    pub members: Vec<u32>,
    pub centre: Vec<f32>,
    /// The mean similarity of the members to the centre.
    pub cohesion: f32,
    pub children: Vec<Area>,
}

/// The areas of `points`, largest first, each with its sub-areas.
pub fn areas(points: &Points, opts: &AreaOptions) -> Vec<Area> {
    let all: Vec<u32> = (0..points.len() as u32).collect();
    if all.is_empty() {
        return Vec::new();
    }
    let mut rng = Rng::new(opts.seed);
    let split = bisect_until(points, all, opts.areas, opts.min_area, &mut rng);
    let mut top = tidy(points, split, opts.min_area);
    for area in &mut top {
        let mut children = bisect_until(
            points,
            area.members.clone(),
            opts.sub_areas,
            opts.min_sub_area,
            &mut rng,
        );
        // One sub-area is the area itself: it says nothing.
        if children.len() < 2 {
            children.clear();
        }
        sort(&mut children);
        area.children = children;
    }
    sort(&mut top);
    top
}

/// Largest first; equal sizes by their first member.
fn sort(areas: &mut [Area]) {
    areas.sort_by(|a, b| {
        b.members
            .len()
            .cmp(&a.members.len())
            .then(a.members.first().cmp(&b.members.first()))
    });
}

fn area_of(points: &Points, mut members: Vec<u32>) -> Area {
    members.sort_unstable();
    let centre = centre_of(points, &members);
    let cohesion = members
        .iter()
        .map(|&m| dot(points.row(m as usize), &centre))
        .sum::<f32>()
        / members.len().max(1) as f32;
    Area {
        members,
        centre,
        cohesion,
        children: Vec::new(),
    }
}

/// The unit mean of the members; the first member if they cancel out.
fn centre_of(points: &Points, members: &[u32]) -> Vec<f32> {
    let mut centre = vec![0f32; points.dims];
    for &m in members {
        for (c, x) in centre.iter_mut().zip(points.row(m as usize)) {
            *c += x;
        }
    }
    if !normalize(&mut centre)
        && let Some(&first) = members.first()
    {
        centre.copy_from_slice(points.row(first as usize));
    }
    centre
}

/// How far the members are from their centre, summed: what a split of
/// this area could gain.
fn spread(area: &Area) -> f32 {
    area.members.len() as f32 * (1.0 - area.cohesion)
}

fn bisect_until(
    points: &Points,
    members: Vec<u32>,
    target: usize,
    min: usize,
    rng: &mut Rng,
) -> Vec<Area> {
    let mut leaves = vec![(area_of(points, members), true)];
    while leaves.len() < target.max(1) {
        let pick = leaves
            .iter()
            .enumerate()
            .filter(|(_, (a, open))| *open && a.members.len() >= 2 * min.max(1))
            .max_by(|(i, (a, _)), (j, (b, _))| spread(a).total_cmp(&spread(b)).then(j.cmp(i)))
            .map(|(i, _)| i);
        let Some(i) = pick else { break };
        match bisect(points, &leaves[i].0.members, min, rng) {
            Some((left, right)) => {
                leaves[i] = (area_of(points, left), true);
                leaves.push((area_of(points, right), true));
            }
            // Identical points, or no split leaves both halves big enough.
            None => leaves[i].1 = false,
        }
    }
    leaves.into_iter().map(|(a, _)| a).collect()
}

/// The best of a few two-way splits, or `None` when none leaves both
/// halves with at least `min` members.
fn bisect(
    points: &Points,
    members: &[u32],
    min: usize,
    rng: &mut Rng,
) -> Option<(Vec<u32>, Vec<u32>)> {
    let mut best: Option<(f32, Vec<u32>, Vec<u32>)> = None;
    for _ in 0..RESTARTS {
        // Start from a member and the member least like it.
        let a = members[rng.below(members.len())];
        let b = *members.iter().min_by(|&&x, &&y| {
            let (dx, dy) = (
                dot(points.row(x as usize), points.row(a as usize)),
                dot(points.row(y as usize), points.row(a as usize)),
            );
            dx.total_cmp(&dy).then(x.cmp(&y))
        })?;
        let mut centres = [
            points.row(a as usize).to_vec(),
            points.row(b as usize).to_vec(),
        ];
        let mut side = vec![false; members.len()];
        for round in 0..ROUNDS {
            let mut changed = false;
            for (s, &m) in side.iter_mut().zip(members) {
                let row = points.row(m as usize);
                let right = dot(row, &centres[1]) > dot(row, &centres[0]);
                changed |= right != *s;
                *s = right;
            }
            if round > 0 && !changed {
                break;
            }
            for (which, centre) in centres.iter_mut().enumerate() {
                let of: Vec<u32> = members
                    .iter()
                    .zip(&side)
                    .filter(|(_, s)| usize::from(**s) == which)
                    .map(|(m, _)| *m)
                    .collect();
                if !of.is_empty() {
                    *centre = centre_of(points, &of);
                }
            }
        }
        let (mut left, mut right) = (Vec::new(), Vec::new());
        let mut score = 0f32;
        for (&m, &s) in members.iter().zip(&side) {
            score += dot(points.row(m as usize), &centres[usize::from(s)]);
            if s { right.push(m) } else { left.push(m) }
        }
        if left.len() < min.max(1) || right.len() < min.max(1) {
            continue;
        }
        if best.as_ref().is_none_or(|(s, _, _)| score > *s) {
            best = Some((score, left, right));
        }
    }
    best.map(|(_, l, r)| (l, r))
}

/// Every point to its nearest centre, a few rounds. Kept only if no area
/// ends below `min`; otherwise the split stands as it was.
fn tidy(points: &Points, split: Vec<Area>, min: usize) -> Vec<Area> {
    if split.len() < 2 {
        return split;
    }
    let n = points.len();
    let mut centres: Vec<Vec<f32>> = split.iter().map(|a| a.centre.clone()).collect();
    let mut of = vec![0usize; n];
    for (i, area) in split.iter().enumerate() {
        for &m in &area.members {
            of[m as usize] = i;
        }
    }
    for _ in 0..TIDY_ROUNDS {
        let mut changed = false;
        for (p, own) in of.iter_mut().enumerate() {
            let row = points.row(p);
            let mut best = (*own, dot(row, &centres[*own]));
            for (c, centre) in centres.iter().enumerate() {
                let sim = dot(row, centre);
                if sim > best.1 {
                    best = (c, sim);
                }
            }
            changed |= best.0 != *own;
            *own = best.0;
        }
        let mut groups: Vec<Vec<u32>> = vec![Vec::new(); centres.len()];
        for (p, &c) in of.iter().enumerate() {
            groups[c].push(p as u32);
        }
        if groups.iter().any(|g| g.len() < min.max(1)) {
            return split;
        }
        for (centre, group) in centres.iter_mut().zip(&groups) {
            *centre = centre_of(points, group);
        }
        if !changed {
            break;
        }
    }
    let mut groups: Vec<Vec<u32>> = vec![Vec::new(); centres.len()];
    for (p, &c) in of.iter().enumerate() {
        groups[c].push(p as u32);
    }
    groups.into_iter().map(|g| area_of(points, g)).collect()
}

/// Which area each point is in, by the order of `areas`.
pub fn area_of_each(n: usize, areas: &[Area]) -> Vec<u32> {
    let mut of = vec![0u32; n];
    for (i, area) in areas.iter().enumerate() {
        for &m in &area.members {
            of[m as usize] = i as u32;
        }
    }
    of
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::explore::synthetic::planted;

    fn opts(areas: usize) -> AreaOptions {
        AreaOptions {
            areas,
            min_area: 5,
            sub_areas: 3,
            min_sub_area: 3,
            seed: 1,
        }
    }

    #[test]
    fn planted_clusters_are_recovered() {
        let points = planted(8, 60, 64, 0.6, 7);
        let found = areas(&points, &opts(8));
        assert_eq!(found.len(), 8);
        // Purity: each area's members come from one planted group.
        let pure: usize = found
            .iter()
            .map(|a| {
                let mut counts = [0usize; 8];
                for &m in &a.members {
                    counts[m as usize / 60] += 1;
                }
                *counts.iter().max().unwrap()
            })
            .sum();
        assert!(
            pure as f32 / 480.0 >= 0.98,
            "purity {}",
            pure as f32 / 480.0
        );
    }

    #[test]
    fn the_same_seed_gives_the_same_areas() {
        let points = planted(5, 40, 32, 0.8, 3);
        assert_eq!(areas(&points, &opts(5)), areas(&points, &opts(5)));
    }

    #[test]
    fn identical_points_do_not_split() {
        let row = {
            let mut r = vec![1.0, 2.0, 3.0, 4.0];
            normalize(&mut r);
            r
        };
        let points = Points::from_rows(4, &vec![row; 40]);
        let found = areas(&points, &opts(6));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].members.len(), 40);
        assert!(found[0].children.is_empty());
        assert!((found[0].cohesion - 1.0).abs() < 1e-5);
    }

    #[test]
    fn no_area_is_smaller_than_the_floor() {
        // More areas asked for than the floor allows.
        let points = planted(3, 20, 16, 0.9, 11);
        let mut o = opts(12);
        o.min_area = 9;
        let found = areas(&points, &o);
        assert!(found.len() <= 6, "{} areas", found.len());
        assert!(found.iter().all(|a| a.members.len() >= 9));
        assert_eq!(found.iter().map(|a| a.members.len()).sum::<usize>(), 60);
    }

    #[test]
    fn sub_areas_partition_their_area() {
        let points = planted(4, 80, 32, 0.7, 5);
        let found = areas(&points, &opts(4));
        assert!(found.iter().any(|a| !a.children.is_empty()));
        for area in &found {
            if area.children.is_empty() {
                continue;
            }
            let mut inside: Vec<u32> = area
                .children
                .iter()
                .flat_map(|c| c.members.clone())
                .collect();
            inside.sort_unstable();
            assert_eq!(inside, area.members);
            assert!(area.children.iter().all(|c| c.members.len() >= 3));
        }
        // Every point is in exactly one area.
        let of = area_of_each(points.len(), &found);
        for (i, area) in found.iter().enumerate() {
            assert!(area.members.iter().all(|&m| of[m as usize] == i as u32));
        }
    }
}
