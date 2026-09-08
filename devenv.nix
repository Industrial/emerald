{
  inputs,
  pkgs,
  ...
}: {
  imports = [./.cursor/nix];

  name = "project-template";

  # Shared modules from `.cursor/nix` — defaults are off; enable what this repo needs.
  cursor.features.program-moon.enable = true;
  cursor.features.program-lean-ctx.enable = true;
  cursor.features.program-roam-code.enable = true;
  cursor.features.dotenv.enable = true;
  cursor.features.packages-base.enable = true;
  cursor.features.packages-rust-dev.enable = true;
  cursor.features.packages-formatters.enable = true;
  cursor.features.env-python-ld.enable = true;
  cursor.features.languages-javascript.enable = true;

  # Agent harness — the `.cursor` toolchain that `.claude` agents/skills and
  # `.mcp.json` expect on PATH (mirrors idclear/monorepo). Paperclip stays
  # monorepo-only: it needs its own Postgres and process supervision.
  cursor.features.program-claude-code.enable = true;
  cursor.features.program-maestro.enable = true;
  cursor.features.program-assay.enable = true;
  cursor.features.program-context7.enable = true;
  cursor.features.program-omniroute.enable = true;
  cursor.features.program-hermes.enable = true;

  cursor.features.languages-rust = {
    enable = true;
    channel = "stable";
  };

  # uv venv at `.devenv/state/venv` — Serena MCP (`scripts/serena-mcp-wrapper.sh`);
  # see pyproject.toml [dependency-groups].
  cursor.features.languages-python-uv = {
    enable = true;
    syncArguments = [
      "--no-install-project"
      "--group"
      "serena"
    ];
  };

  cursor.features.git-hooks-moon = {
    enable = true;
    # Preserve this repo's Moon gate composition (not the shared ci-* defaults).
    preCommitTargets = ":format :check :lint :test";
    prePushTargets = ":format :check :lint :build :test :audit :check-docs";
  };
  cursor.features.git-hooks-prek.enable = true;

  # Project-only env (shared soft defaults cover CARGO_TERM_COLOR / Moon / nextest).
  env = {
    RUST_BACKTRACE = "1";
    RUSTC_WRAPPER = "sccache";
    # LLVM 21 for the `inkwell`-based codegen backend (plan 16 — the
    # Cranelift-vs-LLVM bake-off spec/COMPILER.md's plan-02 decision
    # record flagged as its own revisit trigger). `llvm-sys` looks for
    # this exact env var name to find `llvm-config` without needing it
    # on PATH.
    LLVM_SYS_211_PREFIX = "${pkgs.llvmPackages_21.llvm.dev}";
  };

  # Project-only packages (beads removed upstream in favor of Maestro).
  packages = [
    inputs.definitively.packages.${pkgs.stdenv.hostPlatform.system}.definitively
    pkgs.llvmPackages_21.llvm
    pkgs.libffi
    pkgs.libxml2
  ];

  # Project-only shell wiring (shared features handle moon-sync, prek install).
  enterShell = ''
    # Hermes: project-local home + seed templates (never overwrite secrets).
    export HERMES_HOME="$PWD/.hermes"
    mkdir -p "$HERMES_HOME"
    for _f in config.yaml SOUL.md; do
      if [ ! -f "$HERMES_HOME/$_f" ] && [ -f ".cursor/nix/features/hermes-agent/templates/$_f" ]; then
        cp ".cursor/nix/features/hermes-agent/templates/$_f" "$HERMES_HOME/$_f"
      fi
    done
    if [ ! -f "$HERMES_HOME/.env" ] && [ -f .cursor/nix/features/hermes-agent/templates/.env.example ]; then
      cp .cursor/nix/features/hermes-agent/templates/.env.example "$HERMES_HOME/.env"
    fi
  '';
}
