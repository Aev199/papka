//! Presentation of a tab's docking destination. Drop semantics use the live hit test.
use std::time::{Duration,Instant};
use crate::{layout::DockZone,layout_motion::{Rect,Tween}};
const SPAN:Duration=Duration::from_millis(crate::motion::TRANSIENT_MS);
#[derive(Default)]
pub(crate) struct DockMotion {
    zone:Option<DockZone>,target:Option<Rect>,from:Option<Rect>,started:Option<Instant>,opacity:Tween,
}
impl DockMotion {
    fn shown(&self,now:Instant)->Option<Rect>{
        let to=self.target?;let from=self.from.unwrap_or(to);
        let t=self.started.map_or(1.,|at|(now.saturating_duration_since(at).as_secs_f32()/SPAN.as_secs_f32()).clamp(0.,1.));let t=crate::motion::ease_out_fraction(t);
        let mix=|a:f32,b:f32|a+(b-a)*t;
        Some(Rect{x:mix(from.x,to.x),y:mix(from.y,to.y),w:mix(from.w,to.w),h:mix(from.h,to.h)})
    }
    pub fn sample(&mut self,zone:Option<DockZone>,now:Instant,animate:bool)->(Option<(Rect,f32,DockZone)>,bool){
        if let Some(zone)=zone {
            let to=rect(zone);
            if self.target!=Some(to) {
                self.from=self.shown(now).or(Some(to));self.target=Some(to);self.started=Some(now);
            }
            self.zone=Some(zone);
        }
        let (opacity,fading)=self.opacity.sample(if zone.is_some(){1.}else{0.},!animate,now,crate::motion::TRANSIENT_MS);
        if !animate {self.from=self.target;self.started=None;}
        let moving=self.started.is_some_and(|at|now.saturating_duration_since(at)<SPAN);
        if !moving {self.started=None;}
        if zone.is_none() && !fading {self.zone=None;self.target=None;self.from=None;self.started=None;return(None,false);}
        (self.shown(now).zip(self.zone).map(|(rect,zone)|(rect,opacity,zone)),fading||moving)
    }
}
fn rect(zone:DockZone)->Rect {
    use DockZone::*;
    match zone {
        Left=>Rect{x:0.,y:0.,w:0.5,h:1.},Right=>Rect{x:0.5,y:0.,w:0.5,h:1.},
        Top=>Rect{x:0.,y:0.,w:1.,h:0.5},Bottom=>Rect{x:0.,y:0.5,w:1.,h:0.5},Center=>Rect{x:0.,y:0.,w:1.,h:1.},
    }
}
pub(crate) fn label(zone:DockZone)->&'static str {
    use DockZone::*;match zone {Left=>"Разделить слева",Right=>"Разделить справа",Top=>"Разделить сверху",Bottom=>"Разделить снизу",Center=>"Перенести вкладку"}
}
#[cfg(test)]mod tests {
    use super::*;
    #[test]fn retargeting_starts_at_the_visible_rect_and_dismissal_completes(){
        let now=Instant::now();let mut m=DockMotion::default();m.sample(Some(DockZone::Left),now,true);
        let mid=now+Duration::from_millis(60);let before=m.sample(Some(DockZone::Left),mid,true).0.unwrap();
        let after=m.sample(Some(DockZone::Right),mid,true).0.unwrap();assert_eq!(before.0,after.0);assert_eq!(before.1,after.1);
        let end=mid+SPAN;let settled=m.sample(Some(DockZone::Right),end,true);assert!(!settled.1);assert_eq!(settled.0.unwrap().0,rect(DockZone::Right));
        assert!(m.sample(None,end,true).1);assert!(m.sample(None,end+SPAN,true).0.is_none());
    }
    #[test]fn reduced_motion_is_immediate_and_repeated_frames_do_not_restart(){
        let now=Instant::now();let mut m=DockMotion::default();let shown=m.sample(Some(DockZone::Bottom),now,false);
        assert_eq!(shown.0.unwrap().1,1.);assert!(!shown.1);
        assert!(!m.sample(Some(DockZone::Bottom),now,true).1);assert!(m.sample(None,now,false).0.is_none());
    }
}
