use crate::app::fs_util::repo_root;
use crate::app::process::run as run_process;
use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;

pub struct CommandStep {
    pub program: &'static str,
    pub arguments: &'static [&'static str],
}

pub fn prepare_wasm_tests() -> Result<()> {
    let tools_node_dir = tools_node_dir()?;
    for step in wasm_preparation_steps() {
        let mut command = Command::new(step.program);
        command.current_dir(&tools_node_dir).args(step.arguments);
        run_process(&mut command).with_context(|| {
            format!(
                "WASM test preparation failed while running {} from {}",
                command_description(&step),
                tools_node_dir.display()
            )
        })?;
    }
    Ok(())
}

pub fn tools_node_dir() -> Result<PathBuf> {
    Ok(repo_root()?.join("tools-node"))
}

pub fn wasm_preparation_steps() -> [CommandStep; 2] {
    [
        CommandStep {
            program: "pnpm",
            arguments: &["install", "--frozen-lockfile", "--prefer-offline"],
        },
        CommandStep {
            program: "pnpm",
            arguments: &["exec", "playwright", "install", "chromium"],
        },
    ]
}

fn command_description(step: &CommandStep) -> String {
    std::iter::once(step.program)
        .chain(step.arguments.iter().copied())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wasm_preparation_uses_the_pinned_tools_node_commands_in_order() {
        let steps = wasm_preparation_steps();

        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].program, "pnpm");
        assert_eq!(
            steps[0].arguments,
            ["install", "--frozen-lockfile", "--prefer-offline"]
        );
        assert_eq!(steps[1].program, "pnpm");
        assert_eq!(
            steps[1].arguments,
            ["exec", "playwright", "install", "chromium"]
        );
    }

    #[test]
    fn wasm_preparation_resolves_tools_node_from_the_repository_root() -> Result<()> {
        assert_eq!(tools_node_dir()?, repo_root()?.join("tools-node"));
        Ok(())
    }
}
