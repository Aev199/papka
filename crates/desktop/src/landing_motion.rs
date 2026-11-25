//! Bounded paint-only flights after manual file ordering. No filesystem work.
use std::{cell::RefCell,collections::{HashMap,HashSet},path::{Path,PathBuf},rc::Rc,time::{Duration,Instant}};
pub(crate) const SPAN:Duration=Duration::from_millis(200);
const LIMIT:usize=8;
#[derive(Clone,Copy,Debug,PartialEq)]
pub(crate) struct Rect {pub x:f32,pub y:f32,pub w:f32,pub h:f32}
impl Rect {
    fn valid(self)->bool {[self.x,self.y,self.w,self.h].iter().all(|v|v.is_finite())&&self.w>0.&&self.h>0.}
}
struct Flight {
    folder:PathBuf,allowed:HashSet<PathBuf>,origin:(f32,f32),started:Instant,
    targets:HashMap<PathBuf,Rect>,committed:bool,
}
#[derive(Default)]
struct State {flights:HashMap<u64,Flight>,viewports:HashMap<u64,Rect>}
#[derive(Clone,Default)]
pub(crate) struct Landings(Rc<RefCell<State>>);
pub(crate) struct Ghost {pub path:PathBuf,pub rect:Rect,pub alpha:f32,pub committed:bool}
impl Landings {
    pub(crate) fn begin(&self,side:u64,folder:&Path,paths:&[PathBuf],origin:(f32,f32),now:Instant,committed:bool){
        if !origin.0.is_finite()||!origin.1.is_finite(){return;}
        let allowed=paths.iter().take(LIMIT).cloned().collect::<HashSet<_>>();
        if allowed.is_empty(){return;}
        self.0.borrow_mut().flights.insert(side,Flight{folder:folder.to_path_buf(),allowed,origin,started:now,targets:HashMap::new(),committed});
    }
    pub(crate) fn cancel(&self,side:u64){self.0.borrow_mut().flights.remove(&side);}
    pub(crate) fn record(&self,side:u64,folder:&Path,path:&Path,rect:Rect){
        if !rect.valid(){return;}
        let mut state=self.0.borrow_mut();
        if let Some(flight)=state.flights.get_mut(&side).filter(|f|f.folder==folder&&f.allowed.contains(path)){
            flight.targets.insert(path.to_path_buf(),rect);
        }
    }
    pub(crate) fn viewport(&self,side:u64,rect:Rect){
        if !rect.valid(){return;}
        let mut state=self.0.borrow_mut();
        if state.viewports.get(&side).is_some_and(|old|
            (old.x-rect.x).abs().max((old.y-rect.y).abs()).max((old.w-rect.w).abs()).max((old.h-rect.h).abs())>1.){
            state.flights.remove(&side);
        }
        state.viewports.insert(side,rect);
    }
    pub(crate) fn prune(&self,ids:&[u64]){
        let mut state=self.0.borrow_mut();
        state.flights.retain(|id,_|ids.contains(id));state.viewports.retain(|id,_|ids.contains(id));
    }
    pub(crate) fn advance(&self,now:Instant,animate:bool)->bool{
        let mut state=self.0.borrow_mut();
        state.flights.retain(|_,f|animate&&now.saturating_duration_since(f.started)<crate::motion_policy::diagnostic_span(SPAN));
        !state.flights.is_empty()
    }
    pub(crate) fn ghosts(&self,side:u64,folder:&Path,now:Instant)->Vec<Ghost>{
        let state=self.0.borrow();let Some(f)=state.flights.get(&side).filter(|f|f.folder==folder)else{return Vec::new();};
        let Some(view)=state.viewports.get(&side)else{return Vec::new();};
        let t=(now.saturating_duration_since(f.started).as_secs_f32()/crate::motion_policy::diagnostic_span(SPAN).as_secs_f32()).clamp(0.,1.);
        if t>=1.{return Vec::new();}
        let t=crate::motion_policy::ease_out_fraction(t);
        let mut result=f.targets.iter().filter_map(|(path,target)|{
            if target.x+target.w<=view.x||target.x>=view.x+view.w||target.y+target.h<=view.y||target.y>=view.y+view.h{return None;}
            let mix=|a:f32,b:f32|a+(b-a)*t;
            Some(Ghost{path:path.clone(),rect:Rect{x:mix(f.origin.0+14.,target.x)-view.x,
                y:mix(f.origin.1+14.,target.y)-view.y,w:mix(target.w.min(180.),target.w),h:mix(28.,target.h)},
                alpha:(1.-t).min(0.25)*4.,committed:f.committed})
        }).collect::<Vec<_>>();
        result.sort_by(|a,b|a.path.cmp(&b.path));result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn rect()->Rect{Rect{x:0.,y:0.,w:400.,h:300.}}
    #[test]
    fn flights_follow_real_targets_and_ignore_other_folders_and_unselected_paths(){
        let now=Instant::now();let l=Landings::default();let folder=Path::new("a");let path=folder.join("file");
        l.viewport(1,rect());l.begin(1,folder,&[path.clone()],(100.,100.),now,true);
        l.record(1,Path::new("b"),&path,rect());assert!(l.ghosts(1,folder,now).is_empty());
        l.record(1,folder,&folder.join("other"),rect());assert!(l.ghosts(1,folder,now).is_empty());
        l.record(1,folder,&path,Rect{x:20.,y:30.,w:200.,h:24.});
        let first=l.ghosts(1,folder,now);assert_eq!(first[0].rect.x,114.);assert_eq!(first[0].rect.y,114.);
        assert!(first[0].committed);
        let middle=l.ghosts(1,folder,now+SPAN/2);assert!(middle[0].rect.x>20.&&middle[0].rect.x<114.);
        assert!(l.ghosts(1,Path::new("b"),now).is_empty());assert!(l.ghosts(1,folder,now+SPAN).is_empty());
        assert!(!l.advance(now+SPAN,true));
    }
    #[test]
    fn bounds_resize_new_gesture_and_reduced_motion_release_flights(){
        let now=Instant::now();let l=Landings::default();let folder=Path::new("a");
        let paths=(0..100).map(|i|folder.join(i.to_string())).collect::<Vec<_>>();
        l.viewport(1,rect());l.begin(1,folder,&paths,(100.,100.),now,false);
        for path in &paths {l.record(1,folder,path,rect());}
        assert_eq!(l.ghosts(1,folder,now).len(),LIMIT);
        l.viewport(1,Rect{w:500.,..rect()});assert!(l.ghosts(1,folder,now).is_empty());
        l.begin(1,folder,&paths,(0.,0.),now,true);assert!(!l.advance(now,false));
        l.begin(1,folder,&paths,(0.,0.),now,true);l.prune(&[]);assert!(!l.advance(now,true));
    }
}
