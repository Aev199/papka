//! A bounded FLIP presentation for explicit sorting and filtering in list mode.
use std::{collections::HashMap,sync::Arc,time::{Duration,Instant}};
use filemanager_core::browser::Listing;
use crate::row_motion::Row;
const SPAN:Duration=crate::motion::ITEM_REFLOW;
const LIMIT:usize=5000;
#[derive(Clone,Copy)]
struct Place {top:f32,opacity:f32}
pub(crate) struct ListMotion {
    pub listing:Arc<Listing>,order:Vec<usize>,from:HashMap<usize,Place>,started:Instant,total:f32,
}
impl ListMotion {
    pub fn new(listing:Arc<Listing>,before:&[usize],after:&[usize],old:Option<&Self>,now:Instant)->Option<Self>{
        if before==after || before.len().max(after.len())>LIMIT {return None;}
        let previous=old.filter(|m|Arc::ptr_eq(&m.listing,&listing));
        let places=previous.map(|m|m.geometry(now));
        let total=places.as_ref().map_or(before.len()as f32,|(_,total)|*total);
        let shown:HashMap<_,_>=places.map(|(rows,_)|rows.into_iter().map(|r|(r.index,Place{top:r.top,opacity:r.opacity})).collect())
            .unwrap_or_else(||before.iter().enumerate().map(|(p,&i)|(i,Place{top:p as f32,opacity:1.})).collect());
        let from=after.iter().enumerate().map(|(p,&i)|{
            let mut place=shown.get(&i).copied().unwrap_or(Place{top:p as f32,opacity:0.});
            // Distant rows settle directly instead of crossing or blanking the entire viewport.
            if (place.top-p as f32).abs()>12. {place=Place{top:p as f32,opacity:1.};}
            (i,place)
        }).collect();
        Some(Self{listing,order:after.to_vec(),from,started:now,total})
    }
    pub fn active(&self,now:Instant)->bool{now.saturating_duration_since(self.started)<crate::motion_policy::diagnostic_span(SPAN)}
    pub fn matches(&self,order:&[usize])->bool{self.order==order}
    pub fn geometry(&self,now:Instant)->(Vec<Row>,f32){
        let t=(now.saturating_duration_since(self.started).as_secs_f32()/crate::motion_policy::diagnostic_span(SPAN).as_secs_f32()).clamp(0.,1.);let t=crate::motion::ease_out_fraction(t);
        let rows=self.order.iter().enumerate().map(|(p,&index)|{
            let from=self.from[&index];Row{index,top:from.top+(p as f32-from.top)*t,height:1.,opacity:from.opacity+(1.-from.opacity)*t,position:Some(p)}
        }).collect();
        (rows,self.total+(self.order.len()as f32-self.total)*t)
    }
}
#[cfg(test)]mod tests{
    use super::*;
    fn listing()->Arc<Listing>{Arc::new(Listing{entries:vec![],truncated:false})}
    #[test]fn sorting_retargets_from_current_positions_and_finishes_in_logical_order(){
        let now=Instant::now();let l=listing();let m=ListMotion::new(l.clone(),&[0,1,2],&[2,0,1],None,now).unwrap();
        assert_eq!(m.geometry(now).0[0].top,2.);
        let mid=now+SPAN/2;let before=m.geometry(mid);let next=ListMotion::new(l,&[2,0,1],&[1,2,0],Some(&m),mid).unwrap();
        for row in next.geometry(mid).0 {let previous=before.0.iter().find(|r|r.index==row.index).unwrap();assert_eq!(row.top,previous.top);}
        for(p,row)in next.geometry(mid+SPAN).0.iter().enumerate(){assert_eq!(row.top,p as f32);assert_eq!(row.position,Some(p));assert_eq!(row.opacity,1.);}
        assert!(!next.active(mid+SPAN));
    }
    #[test]fn reversing_a_large_list_never_blanks_distant_rows(){
        let now=Instant::now();let before=(0..100).collect::<Vec<_>>();let after=(0..100).rev().collect::<Vec<_>>();
        let m=ListMotion::new(listing(),&before,&after,None,now).unwrap();let rows=m.geometry(now).0;
        assert_eq!(rows[0].top,0.);assert_eq!(rows[0].opacity,1.);
        assert_eq!(rows[50].top,49.);assert_eq!(rows[50].opacity,1.);
    }
    #[test]fn filtering_keeps_file_identity_and_bounds_retained_work(){
        let now=Instant::now();let l=listing();let m=ListMotion::new(l.clone(),&[0,1,2,3],&[1,3],None,now).unwrap();
        let (rows,total)=m.geometry(now);assert_eq!(total,4.);assert_eq!(rows[1].index,3);assert_eq!(rows[1].top,3.);
        assert_eq!(m.geometry(now+SPAN).1,2.);
        assert!(ListMotion::new(l.clone(),&[0],&[0],None,now).is_none());
        assert!(ListMotion::new(l,&vec![0;LIMIT+1],&[0],None,now).is_none());
    }
}
