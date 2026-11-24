//! Retained horizontal reveal for Miller columns. External offset changes
//! (wheel, scrollbar, viewport clamp) interrupt instead of fighting the user.
use std::time::{Duration, Instant};

const SPAN: Duration = Duration::from_millis(180);
const TOLERANCE: f32 = 0.75;

#[derive(Default)]
pub(crate) struct Reveal {
    active: Option<Flight>,
}

struct Flight {
    from: f32,
    to: f32,
    last_applied: f32,
    started: Instant,
}

impl Reveal {
    pub(crate) fn begin(&mut self, actual: f32, target: f32, now: Instant, instant: bool) -> bool {
        self.active = if !instant && actual.is_finite() && target.is_finite()
            && (actual-target).abs() > TOLERANCE {
            Some(Flight { from:actual,to:target,last_applied:actual,started:now })
        } else { None };
        self.active.is_some()
    }

    pub(crate) fn sample(&mut self, actual: f32, now: Instant, instant: bool) -> Option<(f32,bool)> {
        let flight = self.active.as_mut()?;
        if !actual.is_finite() || (actual-flight.last_applied).abs() > TOLERANCE {
            self.active = None;
            return None;
        }
        let t = if instant { 1. } else { (now.saturating_duration_since(flight.started).as_secs_f32()
            / SPAN.as_secs_f32()).clamp(0.,1.) };
        let next = flight.from + (flight.to-flight.from) * (1.-(1.-t).powi(3));
        flight.last_applied = next;
        let active = t < 1.;
        if !active { self.active = None; }
        Some((next,active))
    }
}

use std::{collections::HashMap,path::{Path,PathBuf}};

const PRESENCE_SPAN: Duration = Duration::from_millis(160);
const DEPARTURE_LIMIT: usize = 8;

#[derive(Clone)]
struct Column {
    path:PathBuf, x:f32, width:f32,
}

struct Exit { column:Column, at:Instant, alpha:f32 }
struct Enter { at:Instant, from:f32 }
impl Enter {
    fn alpha(&self,now:Instant)->f32 {self.from+(1.-self.from)*presence_alpha(now,self.at)}
}

pub(crate) struct Departure {
    pub path:PathBuf, pub x:f32, pub width:f32, pub alpha:f32,
}

/// Column presence is separate from auto-scroll. Only actual path additions
/// and removals animate; viewport changes, other tabs and gestures reset it.
#[derive(Default)]
pub(crate) struct Presence {
    tab:Option<usize>,
    viewport:f32,
    columns:Vec<Column>,
    entering:HashMap<PathBuf,Enter>,
    exiting:Vec<Exit>,
}

impl Presence {
    pub(crate) fn observe(&mut self, tab:usize, folders:&[PathBuf], widths:&[f32],
        viewport:f32, offset:f32, now:Instant, animate:bool) {
        let reset = !animate || self.tab!=Some(tab) || (viewport-self.viewport).abs()>1.;
        let mut x=offset;
        let next:Vec<_>=folders.iter().zip(widths).map(|(path,&width)| {
            let column=Column{path:path.clone(),x,width}; x+=width;column
        }).collect();
        if reset {
            self.entering.clear();self.exiting.clear();
        } else {
            // Sample removed and returning columns before pruning their clocks.
            self.exiting.retain(|exit| now.saturating_duration_since(exit.at)<PRESENCE_SPAN);
            for column in &self.columns {
                if !folders.contains(&column.path) && column.x<viewport && column.x+column.width>0.
                    && self.exiting.len()<DEPARTURE_LIMIT {
                    let alpha=self.entering.get(&column.path).map_or(1.,|enter|enter.alpha(now));
                    self.exiting.push(Exit{column:column.clone(),at:now,alpha});
                }
            }
            for column in &next {
                if !self.columns.iter().any(|old|old.path==column.path) {
                    let from=self.exiting.iter().find(|exit|exit.column.path==column.path)
                        .map_or(0.,|exit|exit.alpha*(1.-presence_alpha(now,exit.at)));
                    self.entering.insert(column.path.clone(),Enter{at:now,from});
                }
            }
            self.entering.retain(|path,enter| folders.contains(path) && now.saturating_duration_since(enter.at)<PRESENCE_SPAN);
            self.exiting.retain(|exit|!folders.contains(&exit.column.path));
        }
        self.tab=Some(tab);self.viewport=viewport;self.columns=next;
    }

    pub(crate) fn entrance(&self,path:&Path,now:Instant)->(f32,f32) {
        let alpha=self.entering.get(path).map_or(1.,|enter|enter.alpha(now));
        (alpha,8.*(1.-alpha))
    }

    pub(crate) fn departures(&self,now:Instant)->Vec<Departure> {
        self.exiting.iter().filter_map(|exit| {
            let t=presence_alpha(now,exit.at);
            (t<1.).then(||Departure{path:exit.column.path.clone(),x:exit.column.x+8.*t,
                width:exit.column.width,alpha:exit.alpha*(1.-t)})
        }).collect()
    }

    pub(crate) fn active(&self,now:Instant)->bool {
        self.entering.values().any(|enter|now.saturating_duration_since(enter.at)<PRESENCE_SPAN)
            ||self.exiting.iter().any(|exit|now.saturating_duration_since(exit.at)<PRESENCE_SPAN)
    }
}

fn presence_alpha(now:Instant,at:Instant)->f32 {
    let t=(now.saturating_duration_since(at).as_secs_f32()/PRESENCE_SPAN.as_secs_f32()).clamp(0.,1.);
    1.-(1.-t).powi(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_retargets_at_current_offset_and_finishes() {
        let now = Instant::now();
        let mut reveal = Reveal::default();
        assert!(reveal.begin(0.,-300.,now,false));
        let at = now+SPAN/2;
        let (middle, active) = reveal.sample(0.,at,false).unwrap();
        assert!(active && middle < 0. && middle > -300.);
        assert!(reveal.begin(middle,-100.,at,false));
        assert_eq!(reveal.sample(middle,at,false),Some((middle,true)));
        assert_eq!(reveal.sample(middle,at+SPAN,false),Some((-100.,false)));
        assert!(reveal.sample(-100.,at+SPAN,false).is_none());
    }

    #[test]
    fn column_presence_only_animates_path_changes_and_drops_returned_ghosts() {
        let now=Instant::now();let mut p=Presence::default();
        let a=PathBuf::from("a");let b=PathBuf::from("a/b");
        p.observe(0,&[a.clone()],&[200.],400.,0.,now,true);
        assert!(!p.active(now));
        p.observe(0,&[a.clone(),b.clone()],&[200.,200.],400.,0.,now,true);
        assert_eq!(p.entrance(&a,now),(1.,0.));
        assert_eq!(p.entrance(&b,now),(0.,8.));
        let later=now+PRESENCE_SPAN;
        p.observe(0,&[a.clone()],&[200.],400.,0.,later,true);
        assert_eq!(p.departures(later).len(),1);
        p.observe(0,&[a.clone(),b.clone()],&[200.,200.],400.,0.,later,true);
        assert!(p.departures(later).is_empty());
        p.observe(1,&[a.clone()],&[200.],400.,0.,later,true);
        assert!(!p.active(later));
        p.observe(1,&[a,b],&[200.,200.],350.,0.,later,true);
        assert!(!p.active(later));
    }

    #[test]
    fn interrupted_column_presence_uses_the_current_opacity() {
        let now=Instant::now();let mut p=Presence::default();
        let a=PathBuf::from("a");let b=PathBuf::from("a/b");
        p.observe(0,&[a.clone()],&[200.],400.,0.,now,true);
        p.observe(0,&[a.clone(),b.clone()],&[200.,200.],400.,0.,now,true);
        let mid=now+PRESENCE_SPAN/4;
        let alpha=p.entrance(&b,mid).0;
        p.observe(0,&[a.clone()],&[200.],400.,0.,mid,true);
        assert_eq!(p.departures(mid)[0].alpha,alpha);
        let back=mid+PRESENCE_SPAN/4;
        let alpha=p.departures(back)[0].alpha;
        p.observe(0,&[a,b.clone()],&[200.,200.],400.,0.,back,true);
        assert_eq!(p.entrance(&b,back).0,alpha);
        assert!(p.departures(back).is_empty());
    }

    #[test]
    fn manual_scroll_cancels_and_reduced_motion_settles() {
        let now = Instant::now();
        let mut reveal = Reveal::default();
        reveal.begin(0.,-200.,now,false);
        assert!(reveal.sample(-20.,now+SPAN/4,false).is_none());
        assert!(reveal.sample(-20.,now+SPAN,false).is_none());
        reveal.begin(-20.,-100.,now,false);
        assert_eq!(reveal.sample(-20.,now,true),Some((-100.,false)));
        assert!(!reveal.begin(0.,-100.,now,true));
    }
}
