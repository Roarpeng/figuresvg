//! Connected components (two-pass union-find, 8-connectivity) with stats,
//! gap-merge across dilations, border-flood-fill hole detection.

use crate::mask::{dilate, Mask};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug)]
pub struct CompStat {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    pub area: i64,
    pub cx: f64,
    pub cy: f64,
}

pub struct Labels {
    pub w: usize,
    pub h: usize,
    pub lab: Vec<i32>, // 0 = background, 1..=count
    pub count: usize,
    pub stats: Vec<CompStat>, // stats[0] = background dummy
}

pub struct UnionFind {
    parent: Vec<i32>,
}

impl UnionFind {
    pub fn new(n: usize) -> UnionFind {
        UnionFind { parent: (0..n as i32).collect() }
    }
    pub fn find(&mut self, i: i32) -> i32 {
        let mut i = i;
        while self.parent[i as usize] != i {
            self.parent[i as usize] = self.parent[self.parent[i as usize] as usize];
            i = self.parent[i as usize];
        }
        i
    }
    pub fn union(&mut self, a: i32, b: i32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra as usize] = rb;
        }
    }
}

/// 8-connected components of a binary mask with bbox/area/centroid stats.
pub fn connected_components(m: &Mask) -> Labels {
    let (w, h) = (m.w, m.h);
    let mut lab = vec![0i32; w * h];
    let mut uf = UnionFind::new(0);
    let mut next_label: i32 = 0;
    let mut roots: [i32; 4] = [0; 4];

    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            if m.bits[row + x] == 0 {
                continue;
            }
            let mut nr = 0;
            if x > 0 && lab[row + x - 1] > 0 {
                roots[nr] = uf.find(lab[row + x - 1]);
                nr += 1;
            }
            if y > 0 {
                let prow = row - w;
                if x > 0 && lab[prow + x - 1] > 0 {
                    roots[nr] = uf.find(lab[prow + x - 1]);
                    nr += 1;
                }
                if lab[prow + x] > 0 {
                    roots[nr] = uf.find(lab[prow + x]);
                    nr += 1;
                }
                if x + 1 < w && lab[prow + x + 1] > 0 {
                    roots[nr] = uf.find(lab[prow + x + 1]);
                    nr += 1;
                }
            }
            if nr == 0 {
                uf.parent.push(next_label);
                lab[row + x] = next_label;
                next_label += 1;
            } else {
                let mut m0 = roots[0];
                for r in &roots[1..nr] {
                    if *r < m0 {
                        m0 = *r;
                    }
                }
                for r in &roots[..nr] {
                    if *r != m0 {
                        uf.parent[*r as usize] = m0;
                    }
                }
                lab[row + x] = m0;
            }
        }
    }

    // compact: map roots to 1..=count in label-id order
    let n_labels = next_label as usize;
    let mut remap = vec![0i32; n_labels];
    let mut nxt: i32 = 1;
    for i in 0..n_labels {
        let r = uf.find(i as i32);
        if remap[r as usize] == 0 {
            remap[r as usize] = nxt;
            nxt += 1;
        }
        remap[i] = remap[r as usize];
    }
    let count = (nxt - 1) as usize;
    for l in lab.iter_mut() {
        if *l != 0 {
            *l = remap[*l as usize];
        }
    }

    // stats
    let mut stats = vec![
        CompStat { x: 0, y: 0, w: 0, h: 0, area: 0, cx: 0.0, cy: 0.0 };
        count + 1
    ];
    let mut acc: Vec<(i32, i32, i32, i32, i64, i64, i64)> =
        vec![(i32::MAX, i32::MAX, i32::MIN, i32::MIN, 0, 0, 0); count + 1];
    for y in 0..h {
        for x in 0..w {
            let l = lab[y * w + x];
            if l == 0 {
                continue;
            }
            let a = &mut acc[l as usize];
            if (x as i32) < a.0 { a.0 = x as i32; }
            if (y as i32) < a.1 { a.1 = y as i32; }
            if (x as i32) > a.2 { a.2 = x as i32; }
            if (y as i32) > a.3 { a.3 = y as i32; }
            a.4 += 1;
            a.5 += x as i64;
            a.6 += y as i64;
        }
    }
    for (i, a) in acc.iter().enumerate().skip(1) {
        let area = a.4;
        stats[i] = CompStat {
            x: a.0,
            y: a.1,
            w: a.2 - a.0 + 1,
            h: a.3 - a.1 + 1,
            area,
            cx: a.5 as f64 / area as f64,
            cy: a.6 as f64 / area as f64,
        };
    }

    Labels { w, h, lab, count, stats }
}

/// Port of merge_across_gaps: union-find merge of components whose
/// dilations (radius `radius`) overlap. Returns new labels + count.
pub fn merge_across_gaps(l: &Labels, radius: i32) -> (Vec<i32>, usize) {
    if l.count <= 2 {
        return (l.lab.clone(), l.count);
    }
    let (w, h) = (l.w, l.h);
    let mut binary = Mask::new(w, h);
    for (i, &v) in l.lab.iter().enumerate() {
        binary.bits[i] = (v > 0) as u8;
    }
    let dil = dilate(&binary, radius);
    let dlab = connected_components(&dil);

    // member original labels per dilated component
    let mut groups: Vec<HashSet<i32>> = vec![HashSet::new(); dlab.count + 1];
    for i in 0..w * h {
        let o = l.lab[i];
        if o > 0 {
            groups[dlab.lab[i] as usize].insert(o);
        }
    }
    let mut uf = UnionFind::new(l.count + 1);
    for g in &groups {
        let mut members: Vec<i32> = g.iter().copied().collect();
        members.sort_unstable();
        if members.len() > 1 {
            for &mbr in &members[1..] {
                uf.union(members[0], mbr);
            }
        }
    }
    let mut remap = vec![0i32; l.count + 1];
    let mut nxt: i32 = 1;
    for i in 1..=l.count {
        let r = uf.find(i as i32);
        if remap[r as usize] == 0 {
            remap[r as usize] = nxt;
            nxt += 1;
        }
        remap[i] = remap[r as usize];
    }
    let count = (nxt - 1) as usize;
    let lab = l.lab.iter().map(|&v| if v > 0 { remap[v as usize] } else { 0 }).collect();
    (lab, count)
}

/// Binary fill of holes: pad by 1, flood-fill background from the border
/// (4-connectivity, like cv2.floodFill default); unreached background is a
/// hole. Returns fg | holes.
pub fn fill_holes(m: &Mask) -> Mask {
    let (w, h) = (m.w, m.h);
    let (pw, ph) = (w + 2, h + 2);
    let mut pad = vec![0u8; pw * ph];
    for y in 0..h {
        for x in 0..w {
            pad[(y + 1) * pw + (x + 1)] = m.bits[y * w + x];
        }
    }
    // BFS flood from (0,0) over background (value 0), marking 2
    let mut stack: Vec<usize> = vec![0];
    pad[0] = 2;
    while let Some(i) = stack.pop() {
        let x = i % pw;
        let y = i / pw;
        let push = |nx: usize, ny: usize, pad: &mut Vec<u8>, stack: &mut Vec<usize>| {
            let j = ny * pw + nx;
            if pad[j] == 0 {
                pad[j] = 2;
                stack.push(j);
            }
        };
        if x > 0 { push(x - 1, y, &mut pad, &mut stack); }
        if x + 1 < pw { push(x + 1, y, &mut pad, &mut stack); }
        if y > 0 { push(x, y - 1, &mut pad, &mut stack); }
        if y + 1 < ph { push(x, y + 1, &mut pad, &mut stack); }
    }
    let mut out = Mask::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let p = pad[(y + 1) * pw + (x + 1)];
            out.bits[y * w + x] = (p != 0) as u8; // fg (1) or hole (2)
        }
    }
    out
}
