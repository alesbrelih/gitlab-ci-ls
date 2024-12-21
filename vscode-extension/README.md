# GitLab CI Language Server (gitlab-ci-ls)

## Disclaimer

This is an independent project and not an official GitLab product.
It is intended to be used alongside `yaml-language-server` (yamlls), providing specialized support for GitLab CI files without replacing yamlls.

## Features

- **Go To Definition**: Navigate to definitions of `jobs`, `includes`, `variables`,
  `needs`, `extends`, `components`, `stages` and `variables`.
- **Find References**: Find all usages of `jobs`, `extends` and `stages`.
- **Autocompletion**: Suggestions for `extends`, `stages`, `needs`, `variables`, `included projects files` and `components`.
- **Hover Information**: View documentation for job with merged definitions.
- **Diagnostics**: Identifies issues with `extends` references, `stage` definitions, `job needs` usage and `components`.
- **Rename**: Supports job renaming.

It also supports jump to included files. In case it is a remote file it tries to downloading using
current workspace git setup and caches it locally.

## Showcase

Note that this video doesn't include all functionalities.

[Watch the video](https://vimeo.com/966578794)

## Configuration

Initialization options:

- **cache**: location for cached remote files
- **log_path**: location for LS log

## Installation

1. **GitHub Releases**: Download from the [GitHub releases page](https://github.com/alesbrelih/gitlab-ci-ls/releases).
2. **Homebrew (macOS)**: `brew install alesbrelih/gitlab-ci-ls/gitlab-ci-ls`
3. **Cargo (Rust Package Manager)**: `cargo install gitlab-ci-ls`
4. **Mason (neovim)**: [Github](https://github.com/williamboman/mason.nvim)

## Build from source

```sh
cargo build --release
```

Executable can then be found at _target/release/gitlab-ci-ls_

## Integration with VSCode

Extension can be found [here](https://marketplace.visualstudio.com/items?itemName=alesbrelih.gitlab-ci-ls).

This extension supports configuration which needs to be set up because _gitlab-ci-ls_
itself isn't installed along with the extension but it needs to be downloaded from
releases, brew or built from source.

![vscode settings](./docs/images/vscode-settings.jpg)
