//! Shared presence clocks. A closing surface paints its last presentation, without input.
use std::time::Instant;
use crate::layout_motion::Tween;
#[derive(Clone,Copy)]
pub(crate) struct Phase {pub alpha:f32,pub closing:bool,pub active:bool}
#[derive(Default)]
pub(crate) struct Clock {tween:Tween,pub closing:bool}
impl Clock {
    pub fn sample(&mut self,open:bool,now:Instant,animate:bool)->Option<Phase>{
        self.closing=!open;
        let (alpha,active)=self.tween.sample(if open{1.}else{0.},!animate,now,120);
        if !open && !active {self.closing=false;return None;}
        Some(Phase{alpha,closing:!open,active})
    }
}
pub(crate) struct Presence<T> {clock:Clock,last:Option<T>,identity:Option<u64>}
impl<T> Default for Presence<T>{fn default()->Self{Self{clock:Clock::default(),last:None,identity:None}}}
impl<T:Clone> Presence<T> {
    pub fn last(&self)->Option<&T>{self.last.as_ref()}
    pub fn retained(&self)->bool{self.last.is_some()}
    pub fn sample(&mut self,requested:Option<T>,now:Instant,animate:bool)->Option<(T,Phase)>{
        self.sample_identity(requested.map(|state|(0,state)),now,animate)
    }
    pub fn sample_identity(&mut self,requested:Option<(u64,T)>,now:Instant,animate:bool)->Option<(T,Phase)>{
        let open=requested.is_some();
        if let Some((identity,state))=requested {
            if self.identity.is_some_and(|old|old!=identity){self.clock=Default::default();}
            self.identity=Some(identity);self.last=Some(state);
        }
        let Some(phase)=self.clock.sample(open,now,animate)else{self.last=None;self.identity=None;return None};
        Some((self.last.as_ref()?.clone(),phase))
    }
}
#[derive(Clone)]
pub(crate) enum Prompt {
    CloseError(String), NewItem{file:bool,path:String},Project{count:usize,error:Option<String>},
    Delete{what:String,permanent:usize,total:usize},
}
#[derive(Clone)]
pub(crate) struct Bulk {
    pub preview:Option<std::sync::Arc<filemanager_core::bulk_rename::RenamePreview>>,
    pub loading:bool,pub error:Option<String>,pub errors:std::sync::Arc<Vec<String>>,
}
#[derive(Clone)]
pub(crate) struct Conflict {
    pub target:String,pub recycle:bool,pub total:usize,pub lines:Vec<(bool,String,String)>,
}
/// Layers are ordered from the topmost popup down to its parents. A closing
/// child blocks its still-live parent; a newly opened child replaces a closing parent.
pub(crate) fn blocks_input(layers:&[(bool,bool)])->bool {
    layers.iter().find(|(live,held)|*live||*held).is_some_and(|(live,_)|!*live)
}
#[derive(Default)]
pub(crate) struct Surfaces {
    pub menu:Presence<crate::ContextMenu>,pub about:Presence<()>,pub prompt:Presence<Prompt>,
    pub comparison:Presence<crate::compare::Comparison>,pub bulk:Presence<Bulk>,pub conflict:Presence<Conflict>,
    pub update:Presence<crate::updates::Phase>,pub transfer:Presence<crate::transfer_panel::TransferPanelPresentation>,
    pub tour:Presence<usize>,pub versions:Presence<crate::version_rules::Editing>,
    pub interface:Clock,
    pub palette:Option<crate::interface::Palette>,pub settings:Option<crate::interface::Settings>,
}
impl Surfaces {
    pub fn retire_interface(&mut self,palette:Option<crate::interface::Palette>,settings:Option<crate::interface::Settings>){
        if palette.is_some(){self.palette=palette;}if settings.is_some(){self.settings=settings;}
    }
}
pub(crate) fn shield()->gpui::Stateful<gpui::Div> {
    use gpui::{prelude::*,div,MouseButton};
    div().id("popup-closing-shield").absolute().inset_0().occlude()
        .on_mouse_down(MouseButton::Left,|_,_,cx|cx.stop_propagation())
        .on_mouse_down(MouseButton::Right,|_,_,cx|cx.stop_propagation())
        .on_mouse_up(MouseButton::Left,|_,_,cx|cx.stop_propagation())
        .on_click(|_,_,cx|cx.stop_propagation())
        .on_scroll_wheel(|_,_,cx|cx.stop_propagation())
}
#[cfg(test)]mod tests {
    use super::*;use std::time::Duration;
    #[test]fn closing_child_blocks_live_parent_but_live_child_replaces_closing_parent(){
        assert!(blocks_input(&[(false,true),(true,true)]));
        assert!(!blocks_input(&[(true,true),(false,true)]));
        assert!(!blocks_input(&[(false,false),(false,false)]));
    }
    #[test]fn large_shared_results_stay_shared_and_are_released_after_closing(){
        let now=Instant::now();let rows=std::sync::Arc::new(vec![1;1000]);let mut p=Presence::default();
        let (shown,_)=p.sample(Some(rows.clone()),now,false).unwrap();assert!(std::sync::Arc::ptr_eq(&shown,&rows));drop(shown);
        p.sample(None,now,true);p.sample(None,now+Duration::from_millis(120),true);
        assert!(!p.retained());assert_eq!(std::sync::Arc::strong_count(&rows),1);
    }
    #[test]fn updates_do_not_restart_and_closing_retains_latest_presentation(){
        let now=Instant::now();let mut p=Presence::default();p.sample(Some(1),now,true);
        let mid=now+Duration::from_millis(60);let before=p.sample(Some(2),mid,true).unwrap();assert!(before.1.alpha>0.);
        let closing=p.sample(None,mid,true).unwrap();assert_eq!(closing.0,2);assert_eq!(closing.1.alpha,before.1.alpha);assert!(closing.1.closing);
        assert!(p.sample(None,mid+Duration::from_millis(120),true).is_none());assert!(p.last.is_none());
    }
    #[test]fn a_new_menu_replays_but_its_async_content_update_does_not(){
        let now=Instant::now();let mut p=Presence::default();p.sample_identity(Some((1,10)),now,true);
        let later=now+Duration::from_millis(150);assert_eq!(p.sample_identity(Some((1,11)),later,true).unwrap().1.alpha,1.);
        assert_eq!(p.sample_identity(Some((2,12)),later,true).unwrap().1.alpha,0.);
    }
    #[test]fn reopening_reverses_from_current_opacity_and_reduced_motion_is_immediate(){
        let now=Instant::now();let mut p=Presence::default();p.sample(Some(1),now,true);
        let mid=now+Duration::from_millis(60);let a=p.sample(None,mid,true).unwrap().1.alpha;
        assert_eq!(p.sample(Some(2),mid,true).unwrap().1.alpha,a);
        assert_eq!(p.sample(Some(3),mid,false).unwrap().1.alpha,1.);assert!(p.sample(None,mid,false).is_none());
    }
}
