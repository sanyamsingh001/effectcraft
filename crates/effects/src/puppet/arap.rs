//! As-rigid-as-possible mesh deformation, after Igarashi, Moscovich & Hughes,
//! "As-Rigid-As-Possible Shape Manipulation" (SIGGRAPH 2005).
//!
//! Two linear least-squares steps per frame:
//!
//! 1. **Similarity step.** Each triangle edge `(i, j)` expresses the third vertex `k` in the
//!    edge's local frame, `v_k = v_i + x·(v_j − v_i) + y·R90(v_j − v_i)`. Minimising the error of
//!    that relation over all three edges of every triangle, with the pinned vertices fixed,
//!    gives a deformation that is locally a similarity (rotation + uniform scale).
//! 2. **Scale-adjustment step.** Each triangle of the step-1 result is fitted with a rigid copy
//!    of its rest shape (2D Procrustes: best rotation about the centroid). The final positions
//!    minimise the difference between every triangle's edge vectors and its fitted edge vectors.
//!
//! Optional extra iterations repeat step 2 on its own result (rotation refinement). Triangle
//! weights stiffen regions (Starch); edge-vector targets implement Bend/Advanced pin rotation
//! and scale. Both systems are symmetric positive definite (a tiny regulariser towards the
//! initial guess keeps under-constrained meshes well-posed) and are solved with Jacobi-
//! preconditioned conjugate gradients on a sparse matrix.

use std::collections::BTreeMap;

/// A soft target for the vector `v_j − v_i`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EdgeTarget {
    pub i: usize,
    pub j: usize,
    pub vec: [f64; 2],
    pub weight: f64,
}

/// Constraints for one solve.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Handles {
    /// Hard position constraints (vertex, position).
    pub fixed: Vec<(usize, [f64; 2])>,
    pub edges: Vec<EdgeTarget>,
    pub points: Vec<PointTarget>,
}

/// A hard position constraint on a point inside a mesh triangle. Weights are finite,
/// nonnegative and sum to one; invalid rows are ignored before any vertex access.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PointTarget {
    pub vertices: [usize; 3],
    pub weights: [f64; 3],
    pub position: [f64; 2],
}
/// Sparse symmetric matrix in row maps (assembly), then CSR for the solve.
struct Sparse {
    rows: Vec<BTreeMap<usize, f64>>,
}

impl Sparse {
    fn new(n: usize) -> Sparse {
        Sparse { rows: vec![BTreeMap::new(); n] }
    }
    fn add(&mut self, i: usize, j: usize, v: f64) {
        *self.rows[i].entry(j).or_insert(0.0) += v;
    }
}

struct Csr {
    ptr: Vec<usize>,
    col: Vec<usize>,
    val: Vec<f64>,
    diag: Vec<f64>,
}

impl Csr {
    fn mul(&self, x: &[f64], y: &mut [f64]) {
        for (r, yr) in y.iter_mut().enumerate() {
            let mut s = 0.0;
            for k in self.ptr[r]..self.ptr[r + 1] {
                s += self.val[k] * x[self.col[k]];
            }
            *yr = s;
        }
    }
}

/// Solve `A x = b` restricted to the free unknowns (`free[i]`), with the fixed unknowns at
/// their values in `x`. Jacobi-preconditioned conjugate gradients, starting from `x`.
fn solve(a: &Sparse, b: &[f64], x: &mut [f64], free: &[bool]) {
    let n = b.len();
    let idx: Vec<usize> = (0..n).filter(|i| free[*i]).collect();
    if idx.is_empty() {
        return;
    }
    let mut pos = vec![usize::MAX; n];
    for (k, i) in idx.iter().enumerate() {
        pos[*i] = k;
    }
    let m = idx.len();
    // Reduced system: A_ff x_f = b_f − A_fc x_c.
    let mut ptr = vec![0];
    let mut col = vec![];
    let mut val = vec![];
    let mut diag = vec![1.0; m];
    let mut rhs = vec![0.0; m];
    for (k, &i) in idx.iter().enumerate() {
        let mut r = b[i];
        for (&j, &v) in &a.rows[i] {
            if free[j] {
                col.push(pos[j]);
                val.push(v);
                if j == i {
                    diag[k] = if v.abs() > 1e-300 { v } else { 1.0 };
                }
            } else {
                r -= v * x[j];
            }
        }
        rhs[k] = r;
        ptr.push(col.len());
    }
    let csr = Csr { ptr, col, val, diag };
    let mut xf: Vec<f64> = idx.iter().map(|i| x[*i]).collect();
    let mut ax = vec![0.0; m];
    csr.mul(&xf, &mut ax);
    let mut r: Vec<f64> = rhs.iter().zip(&ax).map(|(b, a)| b - a).collect();
    let bnorm = rhs.iter().map(|v| v * v).sum::<f64>().sqrt().max(1e-30);
    let mut z: Vec<f64> = r.iter().zip(&csr.diag).map(|(r, d)| r / d).collect();
    let mut p = z.clone();
    let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
    let mut ap = vec![0.0; m];
    for _ in 0..(4 * m + 50) {
        if r.iter().map(|v| v * v).sum::<f64>().sqrt() <= 1e-13 * bnorm {
            break;
        }
        csr.mul(&p, &mut ap);
        let pap: f64 = p.iter().zip(&ap).map(|(a, b)| a * b).sum();
        if pap.abs() < 1e-300 {
            break;
        }
        let alpha = rz / pap;
        for k in 0..m {
            xf[k] += alpha * p[k];
            r[k] -= alpha * ap[k];
        }
        for k in 0..m {
            z[k] = r[k] / csr.diag[k];
        }
        let rz2: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
        let beta = rz2 / rz;
        rz = rz2;
        for k in 0..m {
            p[k] = z[k] + beta * p[k];
        }
    }
    for (k, i) in idx.iter().enumerate() {
        x[*i] = xf[k];
    }
}

const REG: f64 = 1e-9;

/// Deform a mesh: `rest` vertices, CCW `tris` with per-triangle `weights` (1 = normal,
/// larger = stiffer), constraints, and `refine` extra rigid-fit iterations. Returns the
/// deformed vertex positions.
pub fn deform(rest: &[[f64; 2]], tris: &[[usize; 3]], weights: &[f64], h: &Handles, refine: usize) -> Vec<[f64; 2]> {
    let n = rest.len();
    if n == 0 {
        return vec![];
    }
    // Validate point rows before every vertex or scalar-row access. Malformed serialized
    // attachment data does not introduce indexing or non-finite arithmetic into the solve.
    let points: Vec<_> = h.points.iter().filter(|p| valid_point(p, n)).take(32).collect();
    // Initial guess: rest translated by the mean handle displacement.
    let mut t = [0.0, 0.0];
    for (i, p) in &h.fixed {
        t[0] += p[0] - rest[*i][0];
        t[1] += p[1] - rest[*i][1];
    }
    for point in &points {
        for c in 0..2 {
            let rest_point = point.vertices.iter().zip(point.weights).map(|(i, weight)| rest[*i][c] * weight).sum::<f64>();
            t[c] += point.position[c] - rest_point;
        }
    }
    let handle_count = h.fixed.len() + points.len();
    if handle_count > 0 {
        t[0] /= handle_count as f64;
        t[1] /= handle_count as f64;
    }
    if !t.iter().all(|v| v.is_finite()) {
        return rest.to_vec();
    }
    let x0: Vec<[f64; 2]> = rest.iter().map(|p| [p[0] + t[0], p[1] + t[1]]).collect();
    // A translated rest mesh already has zero deformation energy. Preserve that exact
    // solution, especially with a single pin: the similarity system otherwise leaves rotation
    // free and round-off can overwhelm its tiny regularizer (#313). Neutral Bend / Advanced
    // edge targets also fit this pose; rotation or scale targets still require the full solve.
    let close = |a: f64, b: f64| (a - b).abs() <= 16.0 * f64::EPSILON * a.abs().max(b.abs()).max(1.0);
    let positions_fit = h.fixed.iter().all(|(i, p)| x0.get(*i).is_some_and(|q| close(q[0], p[0]) && close(q[1], p[1])));
    let edges_fit = h.edges.iter().all(|e| rest.get(e.i).zip(rest.get(e.j)).is_some_and(|(a, b)| close(b[0] - a[0], e.vec[0]) && close(b[1] - a[1], e.vec[1])));
    let points_fit = points.iter().all(|p| (0..2).all(|c| close(p.vertices.iter().zip(p.weights).map(|(i, w)| x0[*i][c] * w).sum(), p.position[c])));
    if positions_fit && edges_fit && points_fit {
        return x0;
    }
    let mut fixed = vec![false; n];
    for (i, _) in &h.fixed {
        fixed[*i] = true;
    }
    let w = |k: usize| weights.get(k).copied().unwrap_or(1.0);
    let scale2 = mean_edge2(rest, tris).max(1e-12);

    // ---- Step 1: similarity-invariant energy over 2n unknowns (x0, y0, x1, y1, …).
    let mut a = Sparse::new(2 * n);
    let mut b = vec![0.0; 2 * n];
    for (k, t) in tris.iter().enumerate() {
        let wk = w(k);
        for (i, j, l) in [(t[0], t[1], t[2]), (t[1], t[2], t[0]), (t[2], t[0], t[1])] {
            let e = [rest[j][0] - rest[i][0], rest[j][1] - rest[i][1]];
            let d = [rest[l][0] - rest[i][0], rest[l][1] - rest[i][1]];
            let ee = e[0] * e[0] + e[1] * e[1];
            if ee <= 1e-18 {
                continue;
            }
            // R90(u) = (-u.y, u.x).
            let r90 = [-e[1], e[0]];
            let x = (d[0] * e[0] + d[1] * e[1]) / ee;
            let y = (d[0] * r90[0] + d[1] * r90[1]) / ee;
            // residual = B_l v_l + B_i v_i + B_j v_j with 2x2 blocks; R = [[0,-1],[1,0]].
            let bl = [[1.0, 0.0], [0.0, 1.0]];
            let bi = [[x - 1.0, -y], [y, x - 1.0]];
            let bj = [[-x, y], [-y, -x]];
            let blocks = [(l, bl), (i, bi), (j, bj)];
            for (va, ba) in &blocks {
                for (vb, bb) in &blocks {
                    // ba^T bb
                    for r in 0..2 {
                        for c in 0..2 {
                            let v = ba[0][r] * bb[0][c] + ba[1][r] * bb[1][c];
                            if v != 0.0 {
                                a.add(2 * va + r, 2 * vb + c, wk * v);
                            }
                        }
                    }
                }
            }
        }
    }
    for e in &h.edges {
        let ww = e.weight;
        for c in 0..2 {
            a.add(2 * e.i + c, 2 * e.i + c, ww);
            a.add(2 * e.j + c, 2 * e.j + c, ww);
            a.add(2 * e.i + c, 2 * e.j + c, -ww);
            a.add(2 * e.j + c, 2 * e.i + c, -ww);
            b[2 * e.j + c] += ww * e.vec[c];
            b[2 * e.i + c] -= ww * e.vec[c];
        }
    }
    let reg = REG * scale2.recip().min(1.0);
    for i in 0..n {
        for c in 0..2 {
            a.add(2 * i + c, 2 * i + c, reg);
            b[2 * i + c] += reg * x0[i][c];
        }
    }
    let mut x: Vec<f64> = x0.iter().flat_map(|p| [p[0], p[1]]).collect();
    for (i, p) in &h.fixed {
        x[2 * i] = p[0];
        x[2 * i + 1] = p[1];
    }
    let free2: Vec<bool> = (0..2 * n).map(|k| !fixed[k / 2]).collect();
    let points2: Vec<PointRow> =
        points.iter().flat_map(|p| (0..2).map(move |c| (p.vertices.iter().zip(p.weights).map(|(i, w)| (2 * i + c, w)).collect(), p.position[c]))).collect();
    constrained_solve(&a, &b, &mut x, &free2, &points2);
    let mut cur: Vec<[f64; 2]> = (0..n).map(|i| [x[2 * i], x[2 * i + 1]]).collect();

    // ---- Step 2: fit rigid triangles, then match edge vectors (x and y separately).
    let mut l = Sparse::new(n);
    for (k, t) in tris.iter().enumerate() {
        let wk = w(k);
        for (i, j) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            l.add(i, i, wk);
            l.add(j, j, wk);
            l.add(i, j, -wk);
            l.add(j, i, -wk);
        }
    }
    for e in &h.edges {
        l.add(e.i, e.i, e.weight);
        l.add(e.j, e.j, e.weight);
        l.add(e.i, e.j, -e.weight);
        l.add(e.j, e.i, -e.weight);
    }
    for i in 0..n {
        l.add(i, i, reg);
    }
    let free1: Vec<bool> = (0..n).map(|i| !fixed[i]).collect();
    for _ in 0..=refine {
        let mut bx = vec![0.0; n];
        let mut by = vec![0.0; n];
        for (k, t) in tris.iter().enumerate() {
            let wk = w(k);
            let f = fit_rigid([rest[t[0]], rest[t[1]], rest[t[2]]], [cur[t[0]], cur[t[1]], cur[t[2]]]);
            for (a2, b2) in [(0, 1), (1, 2), (2, 0)] {
                let (i, j) = (t[a2], t[b2]);
                let d = [f[b2][0] - f[a2][0], f[b2][1] - f[a2][1]];
                bx[j] += wk * d[0];
                bx[i] -= wk * d[0];
                by[j] += wk * d[1];
                by[i] -= wk * d[1];
            }
        }
        for e in &h.edges {
            bx[e.j] += e.weight * e.vec[0];
            bx[e.i] -= e.weight * e.vec[0];
            by[e.j] += e.weight * e.vec[1];
            by[e.i] -= e.weight * e.vec[1];
        }
        for i in 0..n {
            bx[i] += reg * x0[i][0];
            by[i] += reg * x0[i][1];
        }
        let mut xs: Vec<f64> = cur.iter().map(|p| p[0]).collect();
        let mut ys: Vec<f64> = cur.iter().map(|p| p[1]).collect();
        for (i, p) in &h.fixed {
            xs[*i] = p[0];
            ys[*i] = p[1];
        }
        let points1 =
            |c: usize| points.iter().map(|p| (p.vertices.iter().zip(p.weights).map(|(i, w)| (*i, w)).collect(), p.position[c])).collect::<Vec<PointRow>>();
        constrained_solve(&l, &bx, &mut xs, &free1, &points1(0));
        constrained_solve(&l, &by, &mut ys, &free1, &points1(1));
        cur = (0..n).map(|i| [xs[i], ys[i]]).collect();
    }
    cur
}

fn mean_edge2(rest: &[[f64; 2]], tris: &[[usize; 3]]) -> f64 {
    if tris.is_empty() {
        return 1.0;
    }
    let mut s = 0.0;
    for t in tris {
        let (a, b) = (rest[t[0]], rest[t[1]]);
        s += (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2);
    }
    s / tris.len() as f64
}

/// Best rigid copy of triangle `rest` over triangle `cur` (rotation about the centroids).
pub fn fit_rigid(rest: [[f64; 2]; 3], cur: [[f64; 2]; 3]) -> [[f64; 2]; 3] {
    let cr = [(rest[0][0] + rest[1][0] + rest[2][0]) / 3.0, (rest[0][1] + rest[1][1] + rest[2][1]) / 3.0];
    let cc = [(cur[0][0] + cur[1][0] + cur[2][0]) / 3.0, (cur[0][1] + cur[1][1] + cur[2][1]) / 3.0];
    let (mut sd, mut sc) = (0.0, 0.0);
    for k in 0..3 {
        let p = [rest[k][0] - cr[0], rest[k][1] - cr[1]];
        let q = [cur[k][0] - cc[0], cur[k][1] - cc[1]];
        sd += p[0] * q[0] + p[1] * q[1];
        sc += p[0] * q[1] - p[1] * q[0];
    }
    let ang = sc.atan2(sd);
    let (s, c) = ang.sin_cos();
    let mut out = [[0.0; 2]; 3];
    for k in 0..3 {
        let p = [rest[k][0] - cr[0], rest[k][1] - cr[1]];
        out[k] = [cc[0] + p[0] * c - p[1] * s, cc[1] + p[0] * s + p[1] * c];
    }
    out
}

/// Hard linear point constraint. The row uses original scalar indices.
type PointRow = (Vec<(usize, f64)>, f64);

fn valid_point(point: &PointTarget, vertices: usize) -> bool {
    point.vertices.iter().all(|i| *i < vertices)
        && point.position.iter().all(|v| v.is_finite())
        && point.weights.iter().all(|v| v.is_finite() && *v >= 0.0)
        && (point.weights.iter().sum::<f64>() - 1.0).abs() <= 1e-8
}

// Position attachments are capped at 32 pins; the similarity system has 64 scalar rows.
fn constrained_solve(a: &Sparse, b: &[f64], x: &mut [f64], free: &[bool], points: &[PointRow]) {
    if points.is_empty() {
        return solve(a, b, x, free);
    }
    if x.len() != b.len() || free.len() != b.len() || a.rows.len() != b.len() {
        return;
    }
    let idx: Vec<usize> = (0..b.len()).filter(|i| free[*i]).collect();
    let m = idx.len();
    if m == 0 {
        return;
    }
    let mut pos = vec![usize::MAX; b.len()];
    for (k, i) in idx.iter().enumerate() {
        pos[*i] = k;
    }
    let mut ptr = vec![0];
    let mut col = vec![];
    let mut val = vec![];
    let mut diag = vec![1.0; m];
    let mut rhs = vec![0.0; m];
    for (k, &i) in idx.iter().enumerate() {
        let mut r = b[i];
        for (&j, &v) in &a.rows[i] {
            let Some(is_free) = free.get(j) else {
                return;
            };
            if !v.is_finite() {
                return;
            }
            if *is_free {
                col.push(pos[j]);
                val.push(v);
                if j == i {
                    diag[k] = v;
                }
            } else {
                r -= v * x[j];
            }
        }
        if !r.is_finite() || !diag[k].is_finite() || diag[k] <= 1e-300 {
            return;
        }
        rhs[k] = r;
        ptr.push(col.len());
    }
    let csr = Csr { ptr, col, val, diag };
    let mut rows: Vec<Vec<(usize, f64)>> = vec![];
    let mut targets = vec![];
    let mut lower: Vec<Vec<f64>> = vec![];
    for (row, target) in points.iter().take(64) {
        if !target.is_finite() || row.iter().any(|(i, w)| *i >= x.len() || !w.is_finite()) {
            continue;
        }
        let mut q = *target;
        let mut r = vec![];
        for &(i, w) in row {
            if free[i] {
                r.push((pos[i], w));
            } else {
                q -= w * x[i];
            }
        }
        if !q.is_finite() {
            continue;
        }
        let dot = |other: &Vec<(usize, f64)>| r.iter().map(|(i, w)| w * other.iter().filter(|(j, _)| j == i).map(|(_, v)| *v).sum::<f64>()).sum::<f64>();
        let mut l = vec![0.0; rows.len() + 1];
        for j in 0..rows.len() {
            let mut v = dot(&rows[j]);
            for k in 0..j {
                v -= l[k] * lower[j][k];
            }
            l[j] = v / lower[j][j];
        }
        let d = dot(&r) - l[..rows.len()].iter().map(|v| v * v).sum::<f64>();
        if !d.is_finite() || d <= 1e-14 || !l.iter().all(|v| v.is_finite()) {
            continue;
        }
        l[rows.len()] = d.sqrt();
        rows.push(r);
        targets.push(q);
        lower.push(l);
    }
    if rows.is_empty() {
        return solve(a, b, x, free);
    }
    // P = I − Cᵀ(CCᵀ)⁻¹C projects a search vector into the constraint nullspace.
    // Project the initial guess onto Cx=target, then run PCG with projected residuals
    // and search directions. This keeps each barycentric attachment hard throughout.
    let gram_solve = |mut v: Vec<f64>| -> Option<Vec<f64>> {
        for i in 0..v.len() {
            for j in 0..i {
                v[i] -= lower[i][j] * v[j];
            }
            v[i] /= lower[i][i];
            if !v[i].is_finite() {
                return None;
            }
        }
        for i in (0..v.len()).rev() {
            for j in i + 1..v.len() {
                v[i] -= lower[j][i] * v[j];
            }
            v[i] /= lower[i][i];
            if !v[i].is_finite() {
                return None;
            }
        }
        Some(v)
    };
    let project = |v: &mut [f64]| -> bool {
        let residual: Vec<f64> = rows.iter().map(|r| r.iter().map(|(i, w)| w * v[*i]).sum()).collect();
        let Some(u) = gram_solve(residual) else {
            return false;
        };
        for (r, t) in rows.iter().zip(u) {
            for &(i, w) in r {
                v[i] -= w * t;
            }
        }
        v.iter().all(|v| v.is_finite())
    };
    let mut xf: Vec<f64> = idx.iter().map(|i| x[*i]).collect();
    let residual = rows.iter().zip(&targets).map(|(r, t)| *t - r.iter().map(|(i, w)| w * xf[*i]).sum::<f64>()).collect();
    let Some(correction) = gram_solve(residual) else {
        return;
    };
    for (r, t) in rows.iter().zip(correction) {
        for &(i, w) in r {
            xf[i] += w * t;
        }
    }
    if !xf.iter().all(|v| v.is_finite()) {
        return;
    }
    let mut ax = vec![0.0; m];
    csr.mul(&xf, &mut ax);
    let mut r: Vec<f64> = rhs.iter().zip(&ax).map(|(b, a)| b - a).collect();
    if !project(&mut r) {
        return;
    }
    let norm = |v: &[f64]| v.iter().fold(0.0_f64, |n, v| n.hypot(*v));
    let bnorm = norm(&rhs).max(1e-30);
    if !bnorm.is_finite() {
        return;
    }
    let mut z: Vec<f64> = r.iter().zip(&csr.diag).map(|(r, d)| r / d).collect();
    if !project(&mut z) {
        return;
    }
    let mut p = z.clone();
    let mut rz: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
    let mut ap = vec![0.0; m];
    for _ in 0..(4 * m + 50) {
        if norm(&r) <= 1e-13 * bnorm || !rz.is_finite() || rz.abs() < 1e-300 {
            break;
        }
        csr.mul(&p, &mut ap);
        if !project(&mut ap) {
            return;
        }
        let pap: f64 = p.iter().zip(&ap).map(|(a, b)| a * b).sum();
        if !pap.is_finite() || pap <= 1e-300 {
            break;
        }
        let alpha = rz / pap;
        if !alpha.is_finite() {
            break;
        }
        for k in 0..m {
            xf[k] += alpha * p[k];
            r[k] -= alpha * ap[k];
        }
        if !xf.iter().chain(&r).all(|v| v.is_finite()) {
            return;
        }
        for k in 0..m {
            z[k] = r[k] / csr.diag[k];
        }
        if !project(&mut z) {
            return;
        }
        let rz2: f64 = r.iter().zip(&z).map(|(a, b)| a * b).sum();
        if !rz2.is_finite() {
            break;
        }
        let beta = rz2 / rz;
        if !beta.is_finite() {
            break;
        }
        rz = rz2;
        for k in 0..m {
            p[k] = z[k] + beta * p[k];
        }
        if !p.iter().all(|v| v.is_finite()) {
            return;
        }
    }
    for (k, i) in idx.iter().enumerate() {
        x[*i] = xf[k];
    }
}

/// Test rank without allocating a full mesh matrix. Fixed vertex constraints take priority.
pub(crate) fn independent_point(fixed: &[usize], points: &[PointTarget], new: &PointTarget) -> bool {
    if points.len() >= 32 {
        return false;
    }
    let mut rows: Vec<Vec<(usize, f64)>> = vec![];
    let mut lower: Vec<Vec<f64>> = vec![];
    for point in points.iter().chain(std::iter::once(new)) {
        let row: Vec<(usize, f64)> =
            point.vertices.iter().zip(point.weights).filter(|(i, w)| !fixed.contains(i) && w.abs() > 1e-15).map(|(i, w)| (*i, w)).collect();
        let dot = |other: &Vec<(usize, f64)>| row.iter().map(|(i, w)| w * other.iter().filter(|(j, _)| j == i).map(|(_, v)| *v).sum::<f64>()).sum::<f64>();
        let mut l = vec![0.0; rows.len() + 1];
        for j in 0..rows.len() {
            let mut v = dot(&rows[j]);
            for k in 0..j {
                v -= l[k] * lower[j][k];
            }
            l[j] = v / lower[j][j];
        }
        let d = dot(&row) - l[..rows.len()].iter().map(|v| v * v).sum::<f64>();
        if !d.is_finite() || d <= 1e-14 {
            return false;
        }
        l[rows.len()] = d.sqrt();
        rows.push(row);
        lower.push(l);
    }
    true
}
