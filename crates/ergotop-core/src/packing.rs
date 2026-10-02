//! Gravity (skyline) packing of mempool txs, ported from Ergomempool v1 PackingAlgorithm.js.
use crate::model::TxId;

const V1_NORMALIZE_BYTES: f64 = 20_000.0;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackItem {
    pub id: TxId,
    pub size_bytes: u32,
    pub fee: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Block,
    Overflow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Rect,
    Hexagon,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub id: TxId,
    pub x: u16,
    pub y: u16,
    pub side: u16,
    pub region: Region,
}

#[derive(Clone, Copy, Debug)]
pub struct PackParams {
    pub width: u16,
    pub block_height: u16,
    pub overflow_height: u16,
    pub capacity_bytes: u32,
    pub max_side: u16,
    pub shape: Shape,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PackResult {
    pub placed: Vec<Placed>,
    pub block_bytes: u64,
    pub block_count: usize,
    pub not_shown: usize,
}

pub fn side_for(size_bytes: u32, max_side: u16) -> u16 {
    let norm = (size_bytes as f64 / V1_NORMALIZE_BYTES).min(1.0);
    let max = max_side.max(1) as f64;
    (1.0 + (max - 1.0) * norm.sqrt()).round() as u16
}

fn fee_rate(i: &PackItem) -> u128 {
    i.fee as u128 * 1000 / i.size_bytes.max(1) as u128
}

/// Per-column [lo, hi) vertical bounds of a region.
fn mask(width: u16, height: u16, shape: Shape) -> (Vec<u16>, Vec<u16>) {
    let w = width as usize;
    match shape {
        Shape::Rect => (vec![0; w], vec![height; w]),
        Shape::Hexagon => {
            let q = (width as f64 / 4.0).max(1.0);
            let half = height as f64 / 2.0;
            let mut lo = vec![0; w];
            let mut hi = vec![height; w];
            for x in 0..w {
                let xc = x as f64 + 0.5;
                let inset = if xc < q {
                    (q - xc) / q
                } else if xc > width as f64 - q {
                    (xc - (width as f64 - q)) / q
                } else {
                    0.0
                };
                let margin = (inset * half).ceil() as u16;
                lo[x] = margin.min(height);
                hi[x] = height.saturating_sub(margin).max(lo[x]);
            }
            (lo, hi)
        }
    }
}

/// Places (index, side) items into one region; returns (index, x, y) and the count not placed.
fn pack_region(items: &[(usize, u16)], width: u16, height: u16, shape: Shape) -> (Vec<(usize, u16, u16)>, usize) {
    let (lo, hi) = mask(width, height, shape);
    let mut sky = lo.clone();
    let mut out = Vec::with_capacity(items.len());
    let mut not_shown = 0;
    for &(idx, side) in items {
        if side == 0 || side > width {
            not_shown += 1;
            continue;
        }
        let mut best: Option<(u16, u16)> = None;
        for x in 0..=(width - side) {
            let cols = x as usize..(x + side) as usize;
            let y = cols.clone().map(|c| sky[c]).max().unwrap_or(0);
            let fits = cols.clone().all(|c| y >= lo[c] && y + side <= hi[c]);
            if fits && best.map_or(true, |(by, bx)| (y, x) < (by, bx)) {
                best = Some((y, x));
            }
        }
        match best {
            Some((y, x)) => {
                for c in x as usize..(x + side) as usize {
                    sky[c] = y + side;
                }
                out.push((idx, x, y));
            }
            None => not_shown += 1,
        }
    }
    (out, not_shown)
}

pub fn pack(items: &[PackItem], p: &PackParams) -> PackResult {
    let mut by_rate: Vec<usize> = (0..items.len()).collect();
    by_rate.sort_by(|&a, &b| fee_rate(&items[b]).cmp(&fee_rate(&items[a])).then_with(|| items[a].id.cmp(&items[b].id)));

    let mut block = Vec::new();
    let mut overflow = Vec::new();
    let mut block_bytes: u64 = 0;
    for i in by_rate {
        let size = items[i].size_bytes as u64;
        if block_bytes + size <= p.capacity_bytes as u64 {
            block_bytes += size;
            block.push(i);
        } else {
            overflow.push(i);
        }
    }

    let sized = |idxs: &[usize]| {
        let mut v: Vec<(usize, u16)> = idxs.iter().map(|&i| (i, side_for(items[i].size_bytes, p.max_side))).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| items[a.0].id.cmp(&items[b.0].id)));
        v
    };

    let mut result = PackResult { block_bytes, block_count: block.len(), ..Default::default() };
    let (placed, hidden) = pack_region(&sized(&block), p.width, p.block_height, p.shape);
    result.not_shown += hidden;
    for (i, x, y) in placed {
        result.placed.push(Placed { id: items[i].id.clone(), x, y, side: side_for(items[i].size_bytes, p.max_side), region: Region::Block });
    }
    let (placed, hidden) = pack_region(&sized(&overflow), p.width, p.overflow_height, Shape::Rect);
    result.not_shown += hidden;
    for (i, x, y) in placed {
        result.placed.push(Placed {
            id: items[i].id.clone(),
            x,
            y: y + p.block_height,
            side: side_for(items[i].size_bytes, p.max_side),
            region: Region::Overflow,
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn item(id: &str, size: u32, fee: u64) -> PackItem {
        PackItem { id: id.into(), size_bytes: size, fee }
    }

    fn params(width: u16, block_height: u16, capacity: u32, shape: Shape) -> PackParams {
        PackParams { width, block_height, overflow_height: 20, capacity_bytes: capacity, max_side: 6, shape }
    }

    #[test]
    fn side_follows_v1_curve() {
        assert_eq!(side_for(0, 6), 1);
        assert_eq!(side_for(20_000, 6), 6);
        assert_eq!(side_for(1_000_000, 6), 6);
        assert_eq!(side_for(5_000, 6), 4); // 1 + 5*sqrt(0.25) = 3.5 -> 4
    }

    #[test]
    fn selects_block_by_fee_rate() {
        let items = [item("a", 600, 6000), item("b", 600, 600), item("c", 300, 3000)];
        let r = pack(&items, &params(40, 40, 1000, Shape::Rect));
        let region = |id: &str| r.placed.iter().find(|p| p.id == id).unwrap().region;
        assert_eq!(region("a"), Region::Block);
        assert_eq!(region("c"), Region::Block);
        assert_eq!(region("b"), Region::Overflow);
        assert_eq!(r.block_bytes, 900);
        assert_eq!(r.block_count, 2);
    }

    #[test]
    fn gravity_fills_bottom_row_left_to_right() {
        let items = [item("a", 1, 1), item("b", 1, 1)];
        let p = PackParams { width: 4, block_height: 4, overflow_height: 0, capacity_bytes: 100, max_side: 2, shape: Shape::Rect };
        let r = pack(&items, &p);
        let pos: Vec<(u16, u16)> = r.placed.iter().map(|p| (p.x, p.y)).collect();
        assert_eq!(pos, vec![(0, 0), (1, 0)]);
    }

    #[test]
    fn items_that_do_not_fit_are_counted() {
        let items: Vec<PackItem> = (0..10).map(|i| item(&format!("t{i}"), 20_000, 1)).collect();
        let p = PackParams { width: 6, block_height: 6, overflow_height: 0, capacity_bytes: u32::MAX, max_side: 6, shape: Shape::Rect };
        let r = pack(&items, &p);
        assert_eq!(r.placed.len(), 1);
        assert_eq!(r.not_shown, 9);
    }

    fn overlaps(a: &Placed, b: &Placed) -> bool {
        a.x < b.x + b.side && b.x < a.x + a.side && a.y < b.y + b.side && b.y < a.y + a.side
    }

    proptest! {
        #[test]
        fn packing_invariants(
            sizes in proptest::collection::vec((1u32..30_000, 0u64..100_000), 0..300),
            width in 8u16..120,
            block_height in 8u16..60,
            capacity in 1_000u32..2_000_000,
            hex in any::<bool>(),
        ) {
            let items: Vec<PackItem> = sizes.iter().enumerate().map(|(i, (s, f))| item(&format!("t{i}"), *s, *f)).collect();
            let shape = if hex { Shape::Hexagon } else { Shape::Rect };
            let p = PackParams { width, block_height, overflow_height: 20, capacity_bytes: capacity, max_side: 6, shape };
            let r = pack(&items, &p);
            prop_assert_eq!(r.placed.len() + r.not_shown, items.len());
            prop_assert!(r.block_bytes <= capacity as u64);
            for (i, a) in r.placed.iter().enumerate() {
                prop_assert!(a.x + a.side <= width);
                match a.region {
                    Region::Block => prop_assert!(a.y + a.side <= block_height),
                    Region::Overflow => prop_assert!(a.y >= block_height && a.y + a.side <= block_height + 20),
                }
                for b in &r.placed[i + 1..] {
                    prop_assert!(!overlaps(a, b), "{:?} overlaps {:?}", a, b);
                }
            }
        }
    }
}
