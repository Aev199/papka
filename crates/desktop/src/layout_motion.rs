//! Retained presentation geometry: navigation never starts layout motion.
use std::{collections::HashMap, time::{Duration, Instant}};
use filemanager_core::panes::{Axis, GroupId, Node};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect { pub x: f32, pub y: f32, pub w: f32, pub h: f32 }
impl Rect {
    fn mix(self, to: Self, t: f32) -> Self {
        let v = |a, b| a + (b-a)*t;
        Self { x:v(self.x,to.x), y:v(self.y,to.y), w:v(self.w,to.w), h:v(self.h,to.h) }
    }
    fn collapsed(self, old: Self) -> Self {
        if self.w < old.w - 0.001 {
            Self { x:if self.x+self.w/2. < old.x+old.w/2. {old.x} else {old.x+old.w}, w:0., ..self }
        } else { Self { y:if self.y+self.h/2. < old.y+old.h/2. {old.y} else {old.y+old.h}, h:0., ..self } }
    }
}
#[derive(Clone)]
pub(crate) struct Divider { pub path: Vec<bool>, pub axis: Axis, pub ratio: f32, pub rect: Rect }
pub(crate) fn geometry(node: &Node) -> (HashMap<GroupId, Rect>, Vec<Divider>) {
    fn walk(n:&Node,r:Rect,path:Vec<bool>,out:&mut HashMap<GroupId,Rect>,divs:&mut Vec<Divider>) {
        match n {
            Node::Leaf(id)=>{out.insert(*id,r);}
            Node::Split {axis,ratio,first,second}=>{
                divs.push(Divider {path:path.clone(),axis:*axis,ratio:*ratio,rect:r});
                let (a,b)=if *axis==Axis::Horizontal {
                    (Rect {w:r.w*ratio,..r},Rect {x:r.x+r.w*ratio,w:r.w*(1.-ratio),..r})
                } else {(Rect {h:r.h*ratio,..r},Rect {y:r.y+r.h*ratio,h:r.h*(1.-ratio),..r})};
                let mut p=path.clone();p.push(false);walk(first,a,p,out,divs);
                let mut p=path;p.push(true);walk(second,b,p,out,divs);
            }
        }
    }
    let mut out=HashMap::new();let mut divs=Vec::new();
    walk(node,Rect{x:0.,y:0.,w:1.,h:1.},Vec::new(),&mut out,&mut divs);(out,divs)
}
#[derive(Default)]
pub(crate) struct Tween { from:f32, target:f32, started:Option<Instant> }
impl Tween {
    pub fn sample(&mut self,target:f32,instant:bool,now:Instant,ms:u64)->(f32,bool) {
        let value=match self.started {
            Some(at)=>{let t=(now.saturating_duration_since(at).as_secs_f32()/(ms as f32/1000.)).clamp(0.,1.);self.from+(self.target-self.from)*(1.-(1.-t).powi(3))},
            None=>self.target,
        };
        if instant {self.from=target;self.target=target;self.started=None;return(target,false);}
        if (target-self.target).abs()>0.01 {self.from=value;self.target=target;self.started=Some(now);}
        let active=self.started.is_some_and(|at|now.saturating_duration_since(at)<Duration::from_millis(ms));
        if !active {self.started=None;return(self.target,false);}
        (value,true)
    }
}
pub(crate) enum Drag { Inspector {start:f32,width:f32}, Split {path:Vec<bool>,axis:Axis,start:f32,ratio:f32,span:f32} }
#[derive(Default)]
pub(crate) struct LayoutMotion {
    pub inspector:Tween,
    pub sidebar:Tween,
    pub sidebar_initialized:bool,
    pub closing:Option<GroupId>,
    close_observed:bool,
    pub inspector_width:f32,
    pub drag:Option<Drag>,
    pub bounds:Option<gpui::Bounds<gpui::Pixels>>,
    signature:Option<String>,
    viewport:Option<(u32,u32)>,
    window_size:Option<(u32,u32)>,
    shown:HashMap<GroupId,Rect>,
    from:HashMap<GroupId,Rect>,
    started:Option<Instant>,
    duration:u64,
}
impl LayoutMotion {
    pub fn begin_close(&mut self,id:GroupId) {self.closing=Some(id);self.close_observed=false;}
    pub fn close_finished(&self)->bool {self.close_observed && self.started.is_none()}
    pub fn finish_close(&mut self) {self.closing=None;self.signature=None;self.started=None;}
    pub fn window_changed(&mut self, size:(u32,u32))->bool {
        self.window_size.replace(size).is_some_and(|old|old!=size)
    }
    pub fn sample(&mut self,node:&Node,viewport:(u32,u32),instant:bool,now:Instant)->(HashMap<GroupId,Rect>,bool) {
        let effective=self.closing.and_then(|closing|{
            let ids=geometry(node).0.keys().copied().filter(|id|*id!=closing).collect::<Vec<_>>();node.retain(&ids)
        }).unwrap_or_else(||node.clone());
        let (mut target,_)=geometry(&effective);let sig=format!("{}-{:?}",effective.signature(),self.closing);
        if let Some(id)=self.closing {
            self.close_observed=true;
            if let Some(old)=self.shown.get(&id).copied() {
                // Collapse toward its outer edge; the sibling expands into it.
                let horizontal=target.values().any(|r|r.w>old.w+0.001&&r.y<=old.y&&r.y+r.h>=old.y+old.h);
                let r=if horizontal {Rect{x:if old.x>0. {old.x+old.w}else{old.x},w:0.,..old}}
                    else {Rect{y:if old.y>0. {old.y+old.h}else{old.y},h:0.,..old}};
                target.insert(id,r);
            }
        }
        let instant=instant||self.signature.is_none()||self.viewport!=Some(viewport);
        if instant {self.started=None;self.shown=target.clone();}
        else if self.signature.as_ref()!=Some(&sig) {
            self.from=target.iter().map(|(id,r)|{
                let from=self.shown.get(id).copied().unwrap_or_else(||{
                    let old=self.shown.values().find(|v|r.x+r.w/2.>=v.x&&r.x+r.w/2.<=v.x+v.w&&r.y+r.h/2.>=v.y&&r.y+r.h/2.<=v.y+v.h).copied().unwrap_or(*r);
                    r.collapsed(old)
                });(*id,from)
            }).collect();
            self.duration=if target.len()>self.shown.len(){220}else{140};self.started=Some(now);
        }
        self.signature=Some(sig);self.viewport=Some(viewport);
        let t=self.started.map_or(1.,|at|(now.saturating_duration_since(at).as_secs_f32()/(self.duration as f32/1000.)).clamp(0.,1.));
        let active=t<1.;
        self.shown=target.into_iter().map(|(id,r)|{
            let from=self.from.get(&id).copied().unwrap_or(r);(id,if active {from.mix(r,1.-(1.-t).powi(3))}else{r})
        }).collect();
        if !active {self.started=None;} (self.shown.clone(),active)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn split()->Node {Node::Split{axis:Axis::Horizontal,ratio:0.5,first:Box::new(Node::Leaf(GroupId(1))),second:Box::new(Node::Leaf(GroupId(2)))}}
    #[test] fn panel_reverses_without_jump_and_can_disable_midflight() {
        let now=Instant::now();let mut t=Tween::default();t.sample(300.,false,now,160);
        let time=now+Duration::from_millis(60);let (before,_)=t.sample(300.,false,time,160);
        let (after,active)=t.sample(0.,false,time,160);assert_eq!(before,after);assert!(active);
        assert_eq!(t.sample(0.,true,time,160),(0.,false));assert_eq!(t.sample(0.,false,time,160),(0.,false));
    }
    #[test] fn layout_moves_once_and_keeps_real_rectangles_adjacent() {
        let now=Instant::now();let mut l=LayoutMotion::default();let v=(1000,700);let one=Node::Leaf(GroupId(1));
        l.sample(&one,v,false,now);let (start,active)=l.sample(&split(),v,false,now);assert!(active);assert_eq!(start[&GroupId(2)].w,0.);
        let (mid,_)=l.sample(&split(),v,false,now+Duration::from_millis(80));
        assert!((mid[&GroupId(1)].x+mid[&GroupId(1)].w-mid[&GroupId(2)].x).abs()<0.001);
        let later=now+Duration::from_secs(1);let (end,active)=l.sample(&split(),v,false,later);assert!(!active);assert_eq!(end[&GroupId(1)].w,0.5);
        assert!(!l.sample(&split(),v,false,later).1,"ordinary renders do not restart");
        assert!(!l.sample(&one,(800,700),false,later).1,"window resize settles immediately");
    }
}

#[cfg(test)]
mod resize_tests {
    use super::*;
    #[test]
    fn ratio_drags_are_direct_and_closing_expands_survivor() {
        let now=Instant::now();let v=(1000,700);let mut l=LayoutMotion::default();
        let mut n=Node::Split{axis:Axis::Vertical,ratio:0.5,first:Box::new(Node::Leaf(GroupId(1))),second:Box::new(Node::Leaf(GroupId(2)))};
        l.sample(&n,v,false,now);
        if let Node::Split{ratio,..}=&mut n{*ratio=0.7;}
        let (rects,moving)=l.sample(&n,v,false,now);assert!(!moving);assert_eq!(rects[&GroupId(1)].h,0.7);
        let (rects,moving)=l.sample(&Node::Leaf(GroupId(1)),v,false,now);assert!(moving);assert_eq!(rects[&GroupId(1)].h,0.7);
        let (rects,moving)=l.sample(&Node::Leaf(GroupId(1)),v,true,now);assert!(!moving);assert_eq!(rects[&GroupId(1)].h,1.);
    }
}

#[cfg(test)]
mod close_tests {
    use super::*;
    #[test]
    fn closing_content_stays_until_its_geometry_reaches_the_edge() {
        let now=Instant::now();let v=(1000,700);let mut l=LayoutMotion::default();
        let n=Node::Split{axis:Axis::Horizontal,ratio:0.5,first:Box::new(Node::Leaf(GroupId(1))),second:Box::new(Node::Leaf(GroupId(2)))};
        l.sample(&n,v,false,now);l.begin_close(GroupId(1));assert!(!l.close_finished());
        let (start,active)=l.sample(&n,v,false,now);assert!(active);assert_eq!(start[&GroupId(1)].w,0.5);
        let (mid,_)=l.sample(&n,v,false,now+Duration::from_millis(70));
        assert!(mid[&GroupId(1)].w>0. && mid[&GroupId(1)].w<0.5);
        assert!((mid[&GroupId(1)].w-mid[&GroupId(2)].x).abs()<0.001);
        assert!(!l.close_finished());
        let (end,active)=l.sample(&n,v,false,now+Duration::from_secs(1));assert!(!active);
        assert_eq!(end[&GroupId(1)].w,0.);assert_eq!(end[&GroupId(2)].w,1.);assert!(l.close_finished());
        l.finish_close();assert!(!l.sample(&Node::Leaf(GroupId(2)),v,false,now).1);
    }
}
