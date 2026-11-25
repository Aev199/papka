//! Runs without cfg(test): verifies the real process-local diagnostic scale.
#[path="../crates/desktop/src/motion_policy.rs"] mod motion_policy;
#[path="../crates/desktop/src/column_reveal.rs"] mod column_reveal;
#[path="../crates/desktop/src/mode_motion.rs"] mod mode_motion;
#[path="../crates/desktop/src/spatial_motion.rs"] mod spatial_motion;
#[path="../crates/desktop/src/landing_motion.rs"] mod landing_motion;
mod motion {pub(crate) use crate::motion_policy::{MODE_FADE_OUT,MODE_FADE_IN,ease_out_fraction};}
use std::{path::PathBuf,time::{Duration,Instant}};
fn main(){
    let factor=std::env::var("PAPKA_MOTION_SLOW").unwrap_or_default().parse::<u32>().unwrap_or(1);
    let factor=if [1,2,4].contains(&factor){factor}else{1};
    let now=Instant::now();let span=Duration::from_millis(180);
    assert_eq!(motion_policy::diagnostic_span(span),span*factor);
    let mut reveal=column_reveal::Reveal::default();reveal.begin(0.,-200.,now,false);
    let (x,active)=reveal.sample(0.,now+span/2*factor,false).unwrap();
    assert!(active&&x<0.&&x> -200.);
    assert_eq!(reveal.sample(x,now+span*factor,false),Some((-200.,false)));
    let mut columns=column_reveal::Presence::default();
    let a=PathBuf::from("a");let b=PathBuf::from("a/b");
    columns.observe(0,&[a.clone()],&[200.],400.,0.,now,true);
    columns.observe(0,&[a,b],&[200.,200.],400.,0.,now,true);
    let end=now+Duration::from_millis(160)*factor;
    assert!(columns.active(end-Duration::from_millis(1)));
    assert!(!columns.active(end));
    assert_eq!(motion_policy::entry_appearance(Duration::from_millis(140)*factor,7),(1.,0.));
    use mode_motion::{Mode,ModeMotion};
    let mut mode=ModeMotion::new(Mode::List);mode.begin(Mode::List,Mode::Columns,now,true);
    assert!(mode.sample(now+Duration::from_millis(70)*factor/2,true).2.is_none());
    assert_eq!(mode.sample(now+Duration::from_millis(70)*factor,true).2,Some(Mode::Columns));
    let landing=landing_motion::Landings::default();let folder=std::path::Path::new("a");let path=folder.join("file");
    landing.viewport(1,landing_motion::Rect{x:0.,y:0.,w:400.,h:300.});
    landing.begin(1,folder,&[path.clone()],(100.,100.),now,true);
    landing.record(1,folder,&path,landing_motion::Rect{x:20.,y:30.,w:200.,h:24.});
    let end=now+landing_motion::SPAN*factor;
    assert!(landing.advance(end-Duration::from_millis(1),true));assert!(!landing.advance(end,true));
    use spatial_motion::{Geometry,Position};
    let spatial=spatial_motion::SpatialMotion::default();
    let (old,_)=spatial.frame(folder,now,true);
    old.record_kind(path.clone(),Geometry{origin:Position{x:0.,y:0.},size:Position{x:400.,y:24.}},None,false);
    spatial.begin(folder,now);
    let (destination,_)=spatial.frame(folder,now,true);
    destination.record_kind(path.clone(),Geometry{origin:Position{x:30.,y:30.},size:Position{x:132.,y:148.}},None,false);
    let (_,moving)=spatial.frame(folder,now+span*factor/2,true);assert!(moving);
    let pose=spatial.shared_pose(&path,now+span*factor/2).unwrap();
    assert!(pose.content.icon_size>16.&&pose.content.icon_size<48.);
    assert!(!spatial.frame(folder,now+span*factor,true).1);
    println!("Visual clocks, morph, landing, completion and stagger agree at {factor}x");
}
