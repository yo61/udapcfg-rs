export default {
  extends: ['@commitlint/config-conventional'],
  rules: {
    // `deps` is not a config-conventional type. It is allowed here ahead
    // of the tooling that uses it: go-udap points Dependabot at `deps` so
    // release-please can route bumps to a Dependencies changelog section,
    // since sections are keyed by type and the default `chore(deps)` lands
    // under the hidden `chore` type. Dependabot here updates only the
    // pre-commit hooks (brought forward on 2026-10-15), with its default
    // prefix; `deps` and release-please stay deferred to M5 (OQ-7 in the
    // port spec), so this entry is forward-looking.
    'type-enum': [
      2,
      'always',
      [
        'build',
        'chore',
        'ci',
        'deps',
        'docs',
        'feat',
        'fix',
        'perf',
        'refactor',
        'revert',
        'style',
        'test',
      ],
    ],
  },
};
