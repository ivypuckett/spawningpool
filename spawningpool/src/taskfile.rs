use std::collections::{HashMap, HashSet};
use std::path::Path;

use regex::Regex;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Taskfile {
    #[serde(default)]
    pub tasks: HashMap<String, Task>,
}

#[derive(Debug, Deserialize)]
pub struct Task {
    pub desc: Option<String>,
    #[serde(default)]
    pub vars: HashMap<String, VarValue>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub cmds: Vec<Cmd>,
}

/// A variable value: either a plain string or a shell command.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum VarValue {
    String(String),
    Shell { sh: String },
}

/// A command entry: either a shell string or a task call.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Cmd {
    Shell(String),
    // Retained so task-call command entries (`- task: build`) deserialize;
    // its fields are not read when summarizing a task's referenced variables.
    #[allow(dead_code)]
    TaskCall {
        task: String,
        #[serde(default)]
        vars: HashMap<String, VarValue>,
    },
}

/// A task's referenced variables and description.
#[derive(Debug, PartialEq, Eq)]
pub struct TaskSummary {
    pub vars: Vec<String>,
    pub desc: Option<String>,
}

fn parse_taskfile(path: &Path) -> Result<Taskfile, Box<dyn std::error::Error>> {
    let contents = std::fs::read_to_string(path)?;
    let taskfile = serde_yaml::from_str(&contents)?;
    Ok(taskfile)
}

/// Parse a Taskfile and return a map of task name to its referenced variables
/// (sorted) and description.
pub fn summarize(path: &Path) -> Result<HashMap<String, TaskSummary>, Box<dyn std::error::Error>> {
    let taskfile = parse_taskfile(path)?;
    let summary = taskfile
        .tasks
        .into_iter()
        .map(|(name, task)| {
            let mut vars: Vec<String> = task.referenced_vars().into_iter().collect();
            vars.sort();
            let desc = task.desc;
            (name, TaskSummary { vars, desc })
        })
        .collect();
    Ok(summary)
}

impl Task {
    /// Returns the set of `{{.VAR_NAME}}` references found in this task's
    /// desc, cmds (shell strings only), env values, and vars values.
    pub fn referenced_vars(&self) -> HashSet<String> {
        let re = Regex::new(r"\{\{\.([A-Za-z_][A-Za-z0-9_]*)\}\}").unwrap();
        let mut vars = HashSet::new();

        let mut scan = |s: &str| {
            for cap in re.captures_iter(s) {
                vars.insert(cap[1].to_string());
            }
        };

        if let Some(desc) = &self.desc {
            scan(desc);
        }

        for cmd in &self.cmds {
            if let Cmd::Shell(s) = cmd {
                scan(s);
            }
        }

        for val in self.env.values() {
            scan(val);
        }

        for val in self.vars.values() {
            match val {
                VarValue::String(s) => scan(s),
                VarValue::Shell { sh } => scan(sh),
            }
        }

        vars
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Taskfile {
        serde_yaml::from_str(yaml).unwrap()
    }

    #[test]
    fn task_with_no_vars() {
        let tf = parse(
            r#"
version: '3'
tasks:
  build:
    cmds:
      - cargo build
"#,
        );
        assert!(tf.tasks["build"].referenced_vars().is_empty());
    }

    #[test]
    fn vars_in_cmds() {
        let tf = parse(
            r#"
version: '3'
tasks:
  greet:
    cmds:
      - echo "Hello {{.NAME}}"
"#,
        );
        assert_eq!(
            tf.tasks["greet"].referenced_vars(),
            HashSet::from(["NAME".to_string()])
        );
    }

    #[test]
    fn vars_in_desc_and_env() {
        let tf = parse(
            r#"
version: '3'
tasks:
  deploy:
    desc: "Deploy to {{.ENV}}"
    env:
      TARGET: "{{.REGION}}"
    cmds:
      - ./deploy.sh
"#,
        );
        assert_eq!(
            tf.tasks["deploy"].referenced_vars(),
            HashSet::from(["ENV".to_string(), "REGION".to_string()])
        );
    }

    #[test]
    fn task_call_cmd_not_scanned() {
        let tf = parse(
            r#"
version: '3'
tasks:
  outer:
    cmds:
      - task: inner
        vars:
          FOO: bar
"#,
        );
        // FOO is passed TO inner, not referenced by outer
        assert!(tf.tasks["outer"].referenced_vars().is_empty());
    }

    #[test]
    fn vars_in_task_level_vars() {
        let tf = parse(
            r#"
version: '3'
tasks:
  build:
    vars:
      FULL: "{{.FIRST}}_{{.LAST}}"
    cmds:
      - echo "{{.FULL}}"
"#,
        );
        let refs = tf.tasks["build"].referenced_vars();
        assert!(refs.contains("FIRST"));
        assert!(refs.contains("LAST"));
        assert!(refs.contains("FULL"));
    }

    #[test]
    fn summarize_maps_name_to_vars_and_desc() {
        let yaml = r#"
version: '3'
tasks:
  deploy:
    desc: "Deploy to {{.ENV}}"
    cmds:
      - ./deploy.sh {{.REGION}}
  build:
    cmds:
      - cargo build
"#;
        let path = std::env::temp_dir().join(format!("sp_summarize_{}.yml", std::process::id()));
        std::fs::write(&path, yaml).unwrap();
        let summary = summarize(&path).unwrap();
        std::fs::remove_file(&path).ok();

        assert_eq!(
            summary["deploy"],
            TaskSummary {
                vars: vec!["ENV".to_string(), "REGION".to_string()],
                desc: Some("Deploy to {{.ENV}}".to_string()),
            }
        );
        assert_eq!(
            summary["build"],
            TaskSummary {
                vars: vec![],
                desc: None,
            }
        );
    }
}
