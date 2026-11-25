//! Multi-selection helpers. Paths are kept in display order so batch
//! operations run in the order the user sees.

use std::path::{Path, PathBuf};

/// Paths from `from` to `to` inclusive, in `ordered` order. Empty when
/// either end is not listed.
pub fn range<'a>(ordered: impl IntoIterator<Item = &'a Path>, from: &Path, to: &Path) -> Vec<PathBuf> {
    let ordered: Vec<&Path> = ordered.into_iter().collect();
    let (Some(a), Some(b)) = (
        ordered.iter().position(|path| *path == from),
        ordered.iter().position(|path| *path == to),
    ) else {
        return Vec::new();
    };
    let (start, end) = if a <= b { (a, b) } else { (b, a) };
    ordered[start..=end].iter().map(|path| path.to_path_buf()).collect()
}

/// Adds `path` if absent, removes it otherwise.
pub fn toggle(marked: &mut Vec<PathBuf>, path: &Path) {
    if let Some(index) = marked.iter().position(|p| p == path) {
        marked.remove(index);
    } else {
        marked.push(path.to_path_buf());
    }
}

/// Display order for a folder snapshot under the effective filter.
pub fn visible_indices(entries: &[crate::browser::Entry], sort: crate::sort::SortSpec, show_hidden: bool, filter: &str) -> Vec<usize> {
    visible_indices_with(entries, sort, show_hidden, filter, &|entry: &crate::browser::Entry| Some(entry.size))
}

/// `size` gives measured folder sizes for sorting by size.
pub fn visible_indices_with(entries: &[crate::browser::Entry], sort: crate::sort::SortSpec, show_hidden: bool, filter: &str,
    size: &dyn Fn(&crate::browser::Entry) -> Option<u64>) -> Vec<usize> {
    let filter=filter.to_lowercase();
    let mut indices=crate::sort::sorted_indices_with(entries,sort,size);
    indices.retain(|&i| (show_hidden || !entries[i].hidden) && (filter.is_empty() || entries[i].name.to_lowercase().contains(&filter)));
    indices
}

/// Rows intersecting a vertical marquee, clipped to the list viewport.
/// Scroll is positive distance from the content top; touching an edge is not intersection.
pub fn marquee_rows(count: usize, row_height: f32, start_y: f32, end_y: f32, viewport_height: f32, scroll_y: f32) -> std::ops::Range<usize> {
    if [row_height,start_y,end_y,viewport_height,scroll_y].iter().any(|v| !v.is_finite())
        || row_height<=0. || viewport_height<=0. || scroll_y<0. { return 0..0; }
    let low=start_y.min(end_y).clamp(0.,viewport_height);
    let high=start_y.max(end_y).clamp(0.,viewport_height);
    if low>=high { return 0..0; }
    let start=((low as f64+scroll_y as f64)/row_height as f64).floor() as usize;
    let end=((high as f64+scroll_y as f64)/row_height as f64).ceil() as usize;
    start.min(count)..end.min(count)
}

/// Geometry shared by thumbnail hit testing and rectangular selection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileGrid {
    pub columns: usize,
    pub tile: (f32, f32),
    pub gap: f32,
    pub padding: (f32, f32),
}
impl TileGrid {
    fn valid(self) -> bool {
        self.columns > 0 && self.tile.0 > 0. && self.tile.1 > 0. && self.gap >= 0.
            && [self.tile.0,self.tile.1,self.gap,self.padding.0,self.padding.1].iter().all(|v|v.is_finite())
    }

    /// A tile under the pointer; gutters, gaps and missing final cells are blank.
    pub fn item_at(self,count:usize,point:(f32,f32),scroll:f32)->Option<usize> {
        if !self.valid() || !point.0.is_finite() || !point.1.is_finite() || !scroll.is_finite() || scroll<0. {return None;}
        let x=point.0-self.padding.0;let y=point.1+scroll-self.padding.1;
        if x<0. || y<0. {return None;}
        let stride=(self.tile.0+self.gap,self.tile.1+self.gap);
        let col=(x/stride.0).floor() as usize;let row=(y/stride.1).floor() as usize;
        let index=row.checked_mul(self.columns)?.checked_add(col)?;
        (col<self.columns && index<count && x%stride.0<self.tile.0 && y%stride.1<self.tile.1).then_some(index)
    }

    /// Visible tiles with positive-area intersection, returned in display order.
    pub fn marquee(self,count:usize,start:(f32,f32),end:(f32,f32),viewport:(f32,f32),scroll:f32)->Vec<usize> {
        if !self.valid() || viewport.0<=0. || viewport.1<=0. || scroll<0.
            || [start.0,start.1,end.0,end.1,viewport.0,viewport.1,scroll].iter().any(|v|!v.is_finite()) {return Vec::new();}
        let left=start.0.min(end.0).clamp(0.,viewport.0);let right=start.0.max(end.0).clamp(0.,viewport.0);
        let top=start.1.min(end.1).clamp(0.,viewport.1);let bottom=start.1.max(end.1).clamp(0.,viewport.1);
        if left>=right || top>=bottom {return Vec::new();}
        let stride=(self.tile.0+self.gap,self.tile.1+self.gap);
        let first=((top+scroll-self.padding.1).max(0.)/stride.1).floor() as usize;
        let last=((bottom+scroll-self.padding.1).max(0.)/stride.1).ceil() as usize;
        let first_col=((left-self.padding.0).max(0.)/stride.0).floor() as usize;
        let last_col=(((right-self.padding.0).max(0.)/stride.0).ceil() as usize).min(self.columns);
        let mut chosen=Vec::new();
        for row in first..last.min(count.div_ceil(self.columns)) {
            let y=self.padding.1+row as f32*stride.1-scroll;
            if y>=bottom || y+self.tile.1<=top {continue;}
            for col in first_col..last_col {
                let x=self.padding.0+col as f32*stride.0;let index=row*self.columns+col;
                if index<count && x<right && x+self.tile.0>left {chosen.push(index);}
            }
        }
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn range_is_inclusive_in_either_direction() {
        let list = paths(&["a", "b", "c", "d"]);
        let order = list.iter().map(PathBuf::as_path);
        assert_eq!(range(order.clone(), Path::new("b"), Path::new("d")), paths(&["b", "c", "d"]));
        assert_eq!(range(order.clone(), Path::new("c"), Path::new("a")), paths(&["a", "b", "c"]));
        assert_eq!(range(order.clone(), Path::new("b"), Path::new("b")), paths(&["b"]));
        assert!(range(order, Path::new("x"), Path::new("a")).is_empty());
    }

    #[test]
    fn toggling() {
        let mut marked = paths(&["a"]);
        toggle(&mut marked, Path::new("b"));
        assert_eq!(marked, paths(&["a", "b"]));
        toggle(&mut marked, Path::new("a"));
        assert_eq!(marked, paths(&["b"]));
    }
}
