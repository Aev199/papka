//! Temporary variable-height presentation of a virtualized version stack.
use std::{collections::{HashMap,HashSet},path::PathBuf,sync::Arc,time::{Duration,Instant}};
use filemanager_core::{browser::Listing,versions::Stacked};
const SPAN:Duration=Duration::from_millis(180);
const LIMIT:usize=20_000;
#[derive(Clone,Copy,Debug)]
pub(crate) struct Row {pub index:usize,pub top:f32,pub height:f32,pub position:Option<usize>}
pub(crate) struct RowMotion {
    pub listing:Arc<Listing>,pub roles:Arc<HashMap<PathBuf,Stacked>>,
    order:Vec<usize>,from:HashMap<usize,f32>,target:HashMap<usize,usize>,started:Instant,
}
impl RowMotion {
    pub fn new(listing:Arc<Listing>,roles:Arc<HashMap<PathBuf,Stacked>>,before:&[usize],after:&[usize],now:Instant)->Option<Self>{
        if before==after||before.len().max(after.len())>LIMIT{return None;}
        let mut motion=Self{listing,roles,order:before.to_vec(),from:before.iter().map(|&i|(i,1.)).collect(),target:HashMap::new(),started:now};
        motion.retarget(after,now);Some(motion)
    }
    fn amount(&self,now:Instant)->f32{let t=(now.saturating_duration_since(self.started).as_secs_f32()/SPAN.as_secs_f32()).clamp(0.,1.);1.-(1.-t).powi(3)}
    fn height(&self,index:usize,now:Instant)->f32{let from=self.from.get(&index).copied().unwrap_or(0.);let to=if self.target.contains_key(&index){1.}else{0.};from+(to-from)*self.amount(now)}
    pub fn openness(&self,key:&str,now:Instant)->Option<f32>{
        self.order.iter().find_map(|&i|{
            let path=&self.listing.entries.get(i)?.path;
            matches!(self.roles.get(path),Some(Stacked::Member{key:family,..}) if family==key).then(||self.height(i,now))
        })
    }
    pub fn matches(&self,order:&[usize])->bool{order.len()==self.target.len()&&order.iter().enumerate().all(|(p,i)|self.target.get(i)==Some(&p))}
    pub fn active(&self,now:Instant)->bool{now.saturating_duration_since(self.started)<SPAN}
    pub fn retarget(&mut self,after:&[usize],now:Instant){
        let from=self.order.iter().map(|&i|(i,self.height(i,now))).collect();
        // Insert newly disclosed rows after their preceding retained row.
        let seen=self.order.iter().copied().collect::<HashSet<_>>();
        let mut additions:HashMap<Option<usize>,Vec<usize>>=HashMap::new();let mut previous=None;
        for &index in after {if seen.contains(&index){previous=Some(index);}else{additions.entry(previous).or_default().push(index);}}
        let mut order=additions.remove(&None).unwrap_or_default();
        for &index in &self.order {order.push(index);if let Some(new)=additions.remove(&Some(index)){order.extend(new);}}
        self.order=order;
        self.from=from;self.target=after.iter().enumerate().map(|(p,&i)|(i,p)).collect();self.started=now;
    }
    /// Content coordinates in rows; only a viewport worth of elements is built by the caller.
    pub fn geometry(&self,now:Instant)->(Vec<Row>,f32){
        let mut top=0.;let rows=self.order.iter().map(|&index|{let height=self.height(index,now);let r=Row{index,top,height,position:self.target.get(&index).copied()};top+=height;r}).collect();(rows,top)
    }
}
#[cfg(test)]mod tests{
    use super::*;
    fn listing()->Arc<Listing>{Arc::new(Listing{entries:vec![],truncated:false})}
    #[test]fn closing_keeps_rows_and_moves_neighbors_without_holes(){
        let now=Instant::now();let m=RowMotion::new(listing(),Arc::default(),&[0,1,2,3],&[0,3],now).unwrap();
        let (r,h)=m.geometry(now+SPAN/2);assert!(r[1].height>0.&&r[1].height<1.);assert!(r[1].position.is_none());
        assert_eq!(r[3].top,r[2].top+r[2].height);assert!(h>2.&&h<4.);
        let (r,h)=m.geometry(now+SPAN);assert_eq!(h,2.);assert_eq!(r[3].top,1.);assert!(!m.active(now+SPAN));
    }
    #[test]fn rapid_reversal_starts_from_visible_heights(){
        let now=Instant::now();let mut m=RowMotion::new(listing(),Arc::default(),&[0,3],&[0,1,2,3],now).unwrap();
        let mid=now+SPAN/2;let before=m.geometry(mid);m.retarget(&[0,3],mid);let after=m.geometry(mid);
        assert_eq!(before.1,after.1);for(a,b)in before.0.iter().zip(after.0.iter()){assert_eq!(a.top,b.top);assert_eq!(a.height,b.height);}
        m.retarget(&[0,1,2,3],mid);assert_eq!(m.geometry(mid+SPAN).1,4.);
    }
    #[test]fn unchanged_or_huge_lists_do_not_start(){let now=Instant::now();assert!(RowMotion::new(listing(),Arc::default(),&[0],&[0],now).is_none());assert!(RowMotion::new(listing(),Arc::default(),&vec![0;LIMIT+1],&[0],now).is_none());}
}
