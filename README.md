# GitLab CI Language Server (gitlab-ci-ls)

<p align="center" width="100%">
    <img src="./docs/images/gitlab-ci-ls.png">
</p>

## Disclaimer

This is an independent project and not an official GitLab product.
It is intended to be used alongside `yaml-language-server` (yamlls), providing specialized support for GitLab CI files without replacing yamlls.

## Features

- **Go To Definition**: Navigate to definitions of `jobs`, `includes`, `variables` and `needs`.
- **Find References**: Find all usages of `jobs` and `extends`.
- **Autocompletion**: Suggestions for `extends`, `stages`, `needs`, and `variables`.
- **Hover Information**: View documentation for job with merged definitions.
- **Diagnostics**: Identifies issues with `extends` references and `stage` definitions.

It also supports jump to included files. In case it is a remote file it tries to downloading using
current workspace git setup and caches it locally.

## Configuration

Initialization options:

- **cache**: location for cached remote files
- **log_path**: location for LS log

## Installation

1. **GitHub Releases**: Download from the [GitHub releases page](https://github.com).
2. **Homebrew (macOS)**: `brew install alesbrelih/gitlab-ci-ls/gitlab-ci-ls`
3. **Cargo (Rust Package Manager)**: `cargo install gitlab-ci-ls`

## Build from source

```sh
cargo build --release
```

Executable can then be found at _target/release/gitlab-ci-ls_

## Integration with Neovim

Currently this tool isn't available on Mason [yet](https://github.com/mason-org/mason-registry/pull/5256).

If you want to include it to test it you can use:

```lua
local client = nil

vim.api.nvim_create_autocmd("FileType", {
  pattern = "yaml",
  callback = function(_)
    local root_dir = vim.fs.find(".git", { upward = true, path = vim.fn.expand("%:p:h") })[1]
    vim.notify(root_dir)
    if root_dir then
      root_dir = vim.fn.fnamemodify(root_dir, ":h")
      vim.notify(root_dir)
    else
      root_dir = vim.fn.expand("%:p:h")
    end

    if not client then
      client = vim.lsp.start_client({
        name = "gitlab-ci-ls",
        cmd = { "/path-to-gitlab-ci-ls" },
        init_options = {
          cache = "/path/where/remote/folders/will/be/cached",
          log_path = "/tmp/gitlab-ci-ls.log",
        },
        root_dir = root_dir,
        on_attach = require("lazyvim.plugins.lsp.keymaps").on_attach,
      })

      if not client then
        vim.notify("error creating LSP config")
        return
      end
    end

    vim.lsp.buf_attach_client(0, client)
  end,
})
```

## Integration with VSCode

Extension can be found [here](https://marketplace.visualstudio.com/items?itemName=alesbrelih.gitlab-ci-ls).

This extension supports configuration which needs to be set up because _gitlab-ci-ls_
itself isn't installed along with the extension but it needs to be downloaded from
releases, brew or built from source.

![vscode settings](./docs/images/vscode-settings.jpg)

## Emacs lsp-mode configuration

To use `gitlab-ci-ls` with Emacs `lsp-mode`, reference the below sample
configuration.

```emacs-lisp
(add-to-list 'lsp-language-id-configuration '("\\.gitlab-ci\\.yml$" . "gitlabci"))
(add-to-list 'lsp-language-id-configuration '("/ci-templates/.*\\.yml$" . "gitlabci"))

(lsp-register-custom-settings
  '(("gitlabci.cache" "/path/where/remote/folders/will/be/cached")
    ("gitlabci.log_path" "/tmp/gitlab-ci-ls.log")))

(lsp-register-client
  (make-lsp-client :new-connection (lsp-stdio-connection '("gitlab-ci-ls"))
                  :activation-fn (lsp-activate-on "gitlabci")
                  :server-id 'gitlabci
                  :priority 10
                  :initialization-options (lambda () (gethash "gitlabci" (lsp-configuration-section "gitlabci")))))
```

## TODO

- [ ] Smarter way to initialize, it should support root_dir equal to nil and once file is opened it should receive/calculate new root.
- [x] Fix VSCode completion. It seems it also needs a range to correctly update text.
- [x] Rename to gitlab-ci-ls.
- [x] References for stages
- [ ] Variables can be set in matrixes as well, this is relevant for go to definition on variable.
