//! Retained content coordinates; scrolling does not restart tile movement.
use std::{collections::HashMap, path::{Path,PathBuf}, sync::Arc, time::{Duration,Instant}};
use filemanager_core::browser::Listing;
const SPAN:Duration=Duration::from_millis(180);
#[derive(Clone,Copy,Debug,PartialEq)]
struct Point{x:f32,y:f32}
impl Point{fn mix(self,to:Self,t:f32)->Self{Self{x:self.x+(to.x-self.x)*t,y:self.y+(to.y-self.y)*t}}}
#[derive(Default)]
pub(crate) struct GridMotion{
    folder:PathBuf,indices:Option<Arc<Vec<usize>>>,layout:(usize,u32,u32),
    target:HashMap<PathBuf,Point>,from:HashMap<PathBuf,Point>,start:Option<Instant>,
}
impl GridMotion{
    fn progress(&self,now:Instant)->f32{self.start.map_or(1.,|at|(now.saturating_duration_since(at).as_secs_f32()/SPAN.as_secs_f32()).clamp(0.,1.))}
    fn shown(&self,path:&Path,now:Instant)->Option<Point>{let to=*self.target.get(path)?;let t=self.progress(now);Some(self.from.get(path).copied().unwrap_or(to).mix(to,1.-(1.-t).powi(3)))}
    pub fn prepare(&mut self,folder:&Path,listing:&Listing,indices:Arc<Vec<usize>>,columns:usize,cell:(f32,f32),instant:bool,now:Instant)->bool{
        let layout=(columns,cell.0.to_bits(),cell.1.to_bits());let same_folder=self.folder==folder;
        let changed=!same_folder||self.layout!=layout||self.indices.as_ref().is_none_or(|old|!Arc::ptr_eq(old,&indices));
        if changed{
            // Bound the retained maps, regardless of directory size.
            let target=indices.iter().enumerate().take(5000).map(|(i,&index)|(listing.entries[index].path.clone(),Point{x:(i%columns)as f32*cell.0,y:(i/columns)as f32*cell.1})).collect();
            self.retarget(target,same_folder&&!instant,(cell.0*self.layout.0.max(columns)as f32,cell.1*2.),now);
            self.folder=folder.to_path_buf();self.indices=Some(indices);self.layout=layout;
        }
        if instant||self.progress(now)>=1.{self.start=None;self.from.clear();}self.start.is_some()
    }
    fn retarget(&mut self,target:HashMap<PathBuf,Point>,animate:bool,reach:(f32,f32),now:Instant){
        let from=if animate{target.iter().filter_map(|(path,to)|{let old=self.shown(path,now)?;
            // Allow wrapping across the pane, but settle long vertical moves directly.
            ((old.x-to.x).abs()<=reach.0&&(old.y-to.y).abs()<=reach.1&&old!=*to).then(||(path.clone(),old))
        }).collect::<HashMap<_,_>>()}else{HashMap::new()};
        self.start=(!from.is_empty()).then_some(now);self.from=from;self.target=target;
    }
    pub fn offset(&self,path:&Path,now:Instant)->(f32,f32){let Some(to)=self.target.get(path)else{return(0.,0.)};let shown=self.shown(path,now).unwrap_or(*to);(shown.x-to.x,shown.y-to.y)}
}
#[cfg(test)]mod tests{
    use super::*;
    fn positions(x:f32)->HashMap<PathBuf,Point>{[(PathBuf::from("a"),Point{x,y:0.})].into()}
    #[test]fn retarget_uses_current_position_and_stops_exactly(){
        let now=Instant::now();let mut g=GridMotion::default();g.retarget(positions(0.),false,(100.,100.),now);
        g.retarget(positions(100.),true,(100.,100.),now);assert_eq!(g.offset(Path::new("a"),now),(-100.,0.));
        let mid=now+SPAN/2;let before=g.shown(Path::new("a"),mid);g.retarget(positions(0.),true,(100.,100.),mid);
        assert_eq!(g.shown(Path::new("a"),mid),before);assert_eq!(g.offset(Path::new("a"),mid+SPAN),(0.,0.));
    }
    #[test]fn refresh_does_not_restart_and_navigation_settles(){
        let now=Instant::now();let mut g=GridMotion::default();
        let listing=Listing{entries:(0..6).map(|i|filemanager_core::browser::Entry{path:PathBuf::from(format!("/a/{i}")),name:i.to_string(),is_directory:false,is_symlink:false,hidden:false,size:0,modified:None}).collect(),truncated:false};
        let order=Arc::new((0..6).collect::<Vec<usize>>());let folder=Path::new("/a");
        assert!(!g.prepare(folder,&listing,order.clone(),3,(100.,100.),false,now));
        assert!(g.prepare(folder,&listing,order.clone(),2,(100.,100.),false,now));
        assert_eq!(g.offset(Path::new("/a/2"),now),(200.,-100.));
        let end=now+SPAN;assert!(!g.prepare(folder,&listing,Arc::new((0..6).collect()),2,(100.,100.),false,end));
        assert!(!g.prepare(folder,&listing,order.clone(),2,(100.,100.),false,end));
        assert!(!g.prepare(Path::new("/b"),&listing,order,3,(100.,100.),false,end));assert_eq!(g.offset(Path::new("/a/2"),end),(0.,0.));
    }
    #[test]fn distant_new_and_disabled_tiles_do_not_fly(){
        let now=Instant::now();let mut g=GridMotion::default();g.retarget(positions(0.),false,(100.,100.),now);
        g.retarget(positions(1000.),true,(100.,100.),now);assert!(g.start.is_none());assert_eq!(g.offset(Path::new("new"),now),(0.,0.));
        g.retarget(positions(1050.),false,(100.,100.),now);assert_eq!(g.offset(Path::new("a"),now),(0.,0.));
    }
}
