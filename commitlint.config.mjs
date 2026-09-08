export default {
  extends: ['@commitlint/config-conventional'],
  rules: {
    // `deps` is not a config-conventional type. It is allowed here ahead
    // of the tooling that uses it: go-udap points Dependabot at `deps` so
    // release-please can route bumps to a Dependencies changelog section,
    // since sections are keyed by type and the default `chore(deps)` lands
    // under the hidden `chore` type. Neither is configured in this repo yet
    // -- OQ-7 in the port spec defers both to M5 -- so this entry is
    // forward-looking, not a description of current behaviour.
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
