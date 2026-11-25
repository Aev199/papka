//! Keyboard selection in a thumbnail grid, in logical cell coordinates.
//! Retargeting starts at the drawn position; jumps and layout changes settle.
use std::{path::{Path,PathBuf},time::{Duration,Instant}};

const SPAN:Duration=Duration::from_millis(100);
#[derive(Clone,Copy,Debug,PartialEq)]
pub(crate) struct Point {pub x:f32,pub y:f32}
fn cell(index:usize,columns:usize)->Point {Point{x:(index%columns)as f32,y:(index/columns)as f32}}
struct Flight {scope:(u64,PathBuf,usize),from:Point,to:Point,at:Instant}
#[derive(Default)]
pub(crate) struct Selection {flight:Option<Flight>}
impl Selection {
    pub(crate) fn begin(&mut self,group:u64,folder:&Path,columns:usize,from:usize,to:usize,now:Instant,animate:bool) {
        let columns=columns.max(1);let a=cell(from,columns);let b=cell(to,columns);
        if !animate || from==to || (a.x-b.x).abs()+(a.y-b.y).abs()>1.01 {self.cancel();return;}
        let shown=self.sample(group,folder,columns,now,true).unwrap_or(a);
        self.flight=Some(Flight{scope:(group,folder.to_path_buf(),columns),from:shown,to:b,at:now});
    }
    pub(crate) fn sample(&self,group:u64,folder:&Path,columns:usize,now:Instant,animate:bool)->Option<Point> {
        let f=self.flight.as_ref()?;
        if !animate || f.scope.0!=group || f.scope.1!=folder || f.scope.2!=columns {return None;}
        let elapsed=now.saturating_duration_since(f.at);
        if elapsed>=crate::motion_policy::diagnostic_span(SPAN) {return None;}
        let t=crate::motion_policy::ease_out(elapsed,crate::motion_policy::diagnostic_span(SPAN));
        Some(Point{x:f.from.x+(f.to.x-f.from.x)*t,y:f.from.y+(f.to.y-f.from.y)*t})
    }
    pub(crate) fn active(&mut self,now:Instant,animate:bool)->bool {
        if !animate || self.flight.as_ref().is_some_and(|f|now.saturating_duration_since(f.at)>=crate::motion_policy::diagnostic_span(SPAN)) {self.cancel();}
        self.flight.is_some()
    }
    pub(crate) fn cancel(&mut self) {self.flight=None;}
}

/// Clip the moving plate to a live tile, leaving file hit targets stationary.
pub(crate) fn coverage(at:Point,index:usize,columns:usize,size:(f32,f32),gap:f32)->Option<(f32,f32,f32,f32)> {
    let here=cell(index,columns.max(1));
    let x=(at.x-here.x)*(size.0+gap);let y=(at.y-here.y)*(size.1+gap);
    let left=x.max(0.);let top=y.max(0.);let right=(x+size.0).min(size.0);let bottom=(y+size.1).min(size.1);
    (right>left && bottom>top).then_some((left,top,right-left,bottom-top))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn horizontal_and_vertical_steps_slide_and_finish() {
        let now=Instant::now();let folder=Path::new("folder");let mut s=Selection::default();
        for to in [1,4] {
            s.cancel();s.begin(1,folder,4,0,to,now,true);
            assert_eq!(s.sample(1,folder,4,now,true),Some(Point{x:0.,y:0.}));
            let mid=s.sample(1,folder,4,now+SPAN/2,true).unwrap();
            assert!(mid.x+mid.y>0. && mid.x+mid.y<1.);
            assert!(coverage(mid,0,4,(132.,148.),6.).is_some());
            assert!(coverage(mid,to,4,(132.,148.),6.).is_some());
            assert!(coverage(mid,3,4,(132.,148.),6.).is_none());
            assert!(!s.active(now+SPAN,true));
        }
    }
    #[test]
    fn repeated_and_reversed_keys_retarget_from_the_drawn_point() {
        let now=Instant::now();let folder=Path::new("folder");let mut s=Selection::default();
        s.begin(1,folder,4,0,1,now,true);let mid=now+SPAN/3;
        let drawn=s.sample(1,folder,4,mid,true).unwrap();
        s.begin(1,folder,4,1,0,mid,true);
        assert_eq!(s.sample(1,folder,4,mid,true),Some(drawn));
        assert!(s.sample(2,folder,4,mid,true).is_none());
        assert!(s.sample(1,Path::new("other"),4,mid,true).is_none());
        assert!(s.sample(1,folder,3,mid,true).is_none());
        assert!(!s.active(mid,false));
        s.begin(1,folder,4,0,15,now,true);assert!(!s.active(now,true));
    }
}
