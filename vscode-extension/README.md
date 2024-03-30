# Gitlab CI language server

## **This is not an official language server.**

I've developed this LS to help myself working with Gitlab CI files.

## Installation

**Important:** You will need gitlab-ci-ls installed. You can see the installation options [here](https://github.com/alesbrelih/gitlab-ci-ls).

## Functionalities

Currently it supports only:

- _textDocument/definition_: [Link](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_definition)
- _textDocument/hover_: [Link](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_hover)
- _textDocument/completion_: [Link](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_completion)
- _textDocument/diagnostic_: [Link](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_diagnostic)

### Go To Definition

Both extend and main node keys support go to definition.

```yaml
.base-job:
  something: ...

myjob:
  extends: .base-job
```

In the case above go to definition is supported for _.base-job_ and _myjob_ (if this is just an override of existing job).

For remote file includes it tries to download referenced git repository and
then use its files to jump to definition.

To clone the repository it currently only supports ssh protocol and it
automatically tries to use SSH key in SSH agent.

It will try to find the correct remote by reading current working directory remote.
In case there are multiple remotes (in cases such as forks) it is best to set the remote using the package_map option.

For example:

```
{
  ... other configuration,
  package_map: {
    "mytemplaterepository": "git@gitlab.com"
  }
}
```

in case where we are including gitlab files from a remote. For example:

```yaml
include:
  - project: mytemplaterepository
    ref: 1.0.0
    file:
      - "/.ci-template.yml"
```

Otherwise it will clone from the first remote it has access to which
doesn't guarantee that this is the file version you want.

### Autocomplete

It supports autocompletion for:

- extends
- stages
- variables (currently only root variables, per job definition will be added later on)

### Diagnostic

It shows diagnostics on:

- invalid extends
- invalid stages
