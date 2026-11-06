//! A plan and what its steps do.

use eframe::egui;
use serde_json::Value;

/// What the pointer goes to.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A point of the window, in points from its top left: `{"at": [x, y]}`.
    At([f32; 2]),
    /// One of the app's own, as it names them (`{"ribbon": "New"}`): found
    /// by [`crate::Stage::locate`].
    App(Value),
}

/// One thing a step does.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// The pointer to a target, and a click there unless only `hover`.
    Click {
        target: Target,
        shift: bool,
        hover: bool,
    },
    Key {
        key: egui::Key,
        modifiers: egui::Modifiers,
    },
    /// Text typed into what has the keys, replacing what it holds.
    Type(String),
    Wait(f64),
    /// The view under `at` zoomed by `factor`, as by a pinch there.
    Zoom {
        factor: f32,
        at: Target,
    },
    /// One of the app's own (a tool called, a command run): done by
    /// [`crate::Stage::act`].
    App(Value),
}

/// A step of the plan.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub name: String,
    pub actions: Vec<Action>,
    /// The picture of its end.
    pub shot: Option<String>,
    /// Seconds of video at least: its narration.
    pub seconds: f64,
}

/// The parsed plan.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub fps: f64,
    pub video: bool,
    pub shot_width: u32,
    pub steps: Vec<Step>,
}

fn parse_target(v: &Value) -> Result<Target, String> {
    if !v.is_object() {
        return Err(format!("a target is an object: {v}"));
    }
    let at = v.get("at").and_then(Value::as_array).and_then(|a| {
        a.iter()
            .map(Value::as_f64)
            .collect::<Option<Vec<_>>>()
            .filter(|p| p.len() == 2)
    });
    Ok(match at {
        Some(p) => Target::At([p[0] as f32, p[1] as f32]),
        None => Target::App(v.clone()),
    })
}

fn parse_modifiers(v: &Value) -> egui::Modifiers {
    let on = |k: &str| v.get(k).and_then(Value::as_bool).unwrap_or(false);
    egui::Modifiers {
        shift: on("shift"),
        alt: on("alt"),
        command: on("command"),
        mac_cmd: on("command") && cfg!(target_os = "macos"),
        ctrl: on("command") && !cfg!(target_os = "macos"),
    }
}

fn parse_action(v: &Value) -> Result<Action, String> {
    let o = v
        .as_object()
        .ok_or_else(|| format!("an action is an object: {v}"))?;
    for (key, hover) in [("click", false), ("hover", true)] {
        if let Some(t) = o.get(key) {
            return Ok(Action::Click {
                target: parse_target(t)?,
                shift: o.get("shift").and_then(Value::as_bool).unwrap_or(false),
                hover,
            });
        }
    }
    if let Some(k) = o.get("key").and_then(Value::as_str) {
        let key = egui::Key::from_name(k).ok_or_else(|| format!("no key {k}"))?;
        return Ok(Action::Key {
            key,
            modifiers: parse_modifiers(v),
        });
    }
    if let Some(t) = o.get("type").and_then(Value::as_str) {
        return Ok(Action::Type(t.to_owned()));
    }
    if let Some(f) = o.get("zoom").and_then(Value::as_f64) {
        return Ok(Action::Zoom {
            factor: f as f32,
            at: parse_target(o.get("at").unwrap_or(&Value::Null))?,
        });
    }
    if let Some(s) = o.get("wait").and_then(Value::as_f64) {
        return Ok(Action::Wait(s));
    }
    Ok(Action::App(v.clone()))
}

/// A plan from its JSON: `fps`, `video`, `shot_width` and `steps`, each
/// with `name`, `actions`, `shot` and `seconds`.
///
/// # Errors
///
/// It is no plan, has no steps, or an action of the player's is wrong.
pub fn parse_plan(text: &str) -> Result<Plan, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("the plan: {e}"))?;
    let mut steps = Vec::new();
    for (i, s) in v["steps"].as_array().into_iter().flatten().enumerate() {
        let actions = s["actions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(parse_action)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("step {}: {e}", i + 1))?;
        steps.push(Step {
            name: s["name"]
                .as_str()
                .map_or_else(|| format!("{}", i + 1), str::to_owned),
            actions,
            shot: s["shot"].as_str().map(str::to_owned),
            seconds: s["seconds"].as_f64().unwrap_or(0.0),
        });
    }
    if steps.is_empty() {
        return Err("the plan has no steps".into());
    }
    Ok(Plan {
        fps: v["fps"].as_f64().filter(|f| *f > 0.0).unwrap_or(30.0),
        video: v["video"].as_bool().unwrap_or(true),
        shot_width: v["shot_width"].as_u64().unwrap_or(1600) as u32,
        steps,
    })
}

/// `args` with every string `"$name.key.0"` replaced by that part of the
/// result kept as `name`: for an app's action to use what one before it
/// returned.
///
/// # Errors
///
/// Nothing is kept under the name, or the result has no such part.
pub fn substitute(args: &Value, kept: &serde_json::Map<String, Value>) -> Result<Value, String> {
    Ok(match args {
        Value::String(s) if s.starts_with('$') => {
            let mut path = s[1..].split('.');
            let name = path.next().unwrap_or_default();
            let mut at = kept
                .get(name)
                .ok_or_else(|| format!("{s}: no result kept as {name}"))?;
            for key in path {
                at = match key.parse::<usize>() {
                    Ok(i) => at.get(i),
                    Err(_) => at.get(key),
                }
                .ok_or_else(|| format!("{s}: no {key} in {at}"))?;
            }
            at.clone()
        }
        Value::Array(a) => Value::Array(
            a.iter()
                .map(|x| substitute(x, kept))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| Ok((k.clone(), substitute(x, kept)?)))
                .collect::<Result<_, String>>()?,
        ),
        v => v.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_plan_reads_its_steps_and_actions() {
        let plan = parse_plan(
            r#"{"fps": 25, "steps": [{"name": "base", "seconds": 3.5, "shot": "01.png",
                "actions": [
                  {"tool": "new_sketch", "args": {"plane": "xy"}, "as": "s"},
                  {"click": {"ribbon": "Extrude"}},
                  {"click": {"at": [10, 5]}, "shift": true},
                  {"hover": {"model": [0, 0, 2]}},
                  {"key": "Enter"},
                  {"type": "20"},
                  {"zoom": 0.5, "at": {"sketch": [0, 0]}},
                  {"wait": 0.5}]}]}"#,
        )
        .unwrap();
        assert!((plan.fps - 25.0).abs() < 1e-9);
        assert_eq!(plan.shot_width, 1600);
        let s = &plan.steps[0];
        assert_eq!(s.name, "base");
        assert!((s.seconds - 3.5).abs() < 1e-9);
        assert_eq!(s.actions.len(), 8);
        assert!(matches!(&s.actions[0], Action::App(v) if v["tool"] == "new_sketch"));
        assert_eq!(
            s.actions[2],
            Action::Click {
                target: Target::At([10.0, 5.0]),
                shift: true,
                hover: false
            }
        );
        assert!(matches!(
            &s.actions[3],
            Action::Click { target: Target::App(v), hover: true, .. } if v["model"][2] == 2
        ));
        assert!(parse_plan(r#"{"steps": [{"actions": [{"key": "Nope"}]}]}"#).is_err());
        assert!(parse_plan(r#"{"steps": []}"#).is_err());
    }

    #[test]
    fn arguments_take_results_kept_before() {
        let mut kept = serde_json::Map::new();
        kept.insert("s".into(), json!({"feature": 3, "new_curves": [4, 5]}));
        let args = json!({"feature": "$s.feature", "curves": "$s.new_curves",
                          "first": "$s.new_curves.0", "name": "plain"});
        assert_eq!(
            substitute(&args, &kept).unwrap(),
            json!({"feature": 3, "curves": [4, 5], "first": 4, "name": "plain"})
        );
        assert!(substitute(&json!("$t.feature"), &kept).is_err());
        assert!(substitute(&json!("$s.nothing"), &kept).is_err());
    }
}
