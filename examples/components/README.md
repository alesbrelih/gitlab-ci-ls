# CI/CD components example

A small [component project](https://docs.gitlab.com/ee/ci/components/) for trying out the
component support of `gitlab-ci-ls`. Open this directory as the workspace root in your editor.

It only uses components of the project itself (`$CI_SERVER_FQDN/$CI_PROJECT_PATH/...`), so they
are read from `templates/` and nothing is cloned. The same files are used by the end-to-end tests in
`src/gitlab_ci_ls_parser/component_e2e_tests.rs`.

```
.
├── .gitlab-ci.yml               # includes both components, no diagnostics
├── templates/
│   ├── deploy.yml               # component with all kinds of inputs
│   └── lint/template.yml        # component without a spec header
└── errors/
    ├── invalid-inputs.yml       # wrong component includes
    └── invalid-template.yml     # wrong interpolation inside a template
```

## Things to try

In `.gitlab-ci.yml`:

- Hover over `deploy@...` to get a table with all inputs, or over an input such as `replicas`
  to see its type, default and description.
- Go to definition on the component jumps to `templates/deploy.yml`. On an input it jumps to
  the input in `spec:inputs`.
- Add a new line below `inputs:` and start typing to complete input names that aren't set yet.
- Remove the value of `environment:` or `dry_run:` and trigger completion to get
  `staging`/`production` or `true`/`false`.

In `templates/deploy.yml`:

- Type `$[[ ` to complete `inputs`/`component`, `$[[ inputs.` to complete the declared inputs and
  `$[[ inputs.stage | ` to complete the interpolation functions.
- Hover over or go to the definition of `$[[ inputs.replicas ]]`.

## Diagnostics

`errors/invalid-inputs.yml` reports:

| Line | Problem |
| --- | --- |
| `environment: qa` | not one of the `options` |
| `version: 1.2.3` | doesn't match the `regex` |
| `replicas: three` | not a `number` |
| `dry_run: "true"` | a quoted string isn't a `boolean` |
| `unknown: value` | input isn't declared by the component |
| second `deploy` include | required inputs `environment` and `version` are missing |
| `fix: true` | `lint` doesn't accept any inputs |
| `missing@...` | component can't be resolved (warning) |

`errors/invalid-template.yml` reports `$[[ inputs.typo ]]` (not declared),
`$[[ component.name ]]` (not declared in `spec:component`) and `to_upper` (unknown function).
