# Project Decisions

This is a list of some decisions that the project has made in the past,
especially regarding certain features. If you would like to revisit these
decisions, feel free to start a discussion on GitHub or Discord, but be prepared
for maintainers to still feel the same way. Do not directly send a PR
implementing one of these features without a discussion first.

### AI/LLM Policy

This project does not have an official policy yet (but maybe soon). Please make
sure that you understand and stand by any prose or code that you send to this
community, including in issues, pull requests, or messages.

References: [#9219](https://github.com/jj-vcs/jj/issues/9219),
[#10057](https://github.com/jj-vcs/jj/pull/10057) (pending),
[#10191](https://github.com/jj-vcs/jj/pull/10191) (pending)

### `jj-*` commands in `$PATH`

`jj` will not automatically search `$PATH` for commands with a `jj-` prefix like
Git does (for example, `jj custom` will not run a `jj-custom` binary that you
have). Instead, you should create an
[alias using `jj util exec`](config.md#alias-arbitrary-commands):

```sh
jj config set --user aliases.custom '["util", "exec", "--", "jj-custom"]'
```

Note that this feature may be removed or replaced by an embedded scripting
language in the future.

References: [#3001](https://github.com/jj-vcs/jj/issues/3001)

### Configs for default command options

While `jj` does have some config options that tweak default behavior of commands
(e.g., the `[revsets]` and `[templates]` tables define the default revsets and
templates for some commands, respectively), this functionality does not extend
to all possible options throughout the CLI. For example, there is no setting to
configure a default `--limit` for `jj log`, or to always pass `--interactive` to
`jj squash`, or to have a default of `--onto 'trunk()'` for `jj rebase`, etc.
See the [Configuration docs](config.md) for the supported config options.

Some arguments against having this feature, learned from Mercurial's experience
with deprecating it, are (quoted from @martinvonz in
[#1509](https://github.com/jj-vcs/jj/issues/1509#issuecomment-1504684964)):

- It can be hard to override the defaults. If you make `--reversed` the default
  log order, we'd need to add a `--no-reversed` option for this case.
- It's likely to confuse scripts.
- It's likely to confuse your coworkers when they're trying to help you with
  something on your machine.

However, `jj` does support repeating flags that accept a singular value, with
the last value given overriding all previous ones
([#9861](https://github.com/jj-vcs/jj/pull/9861)). For example,
`jj log --limit 10 --limit 5` is equivalent to `jj log --limit 5`. This means in
some cases you can make aliases with your desired default configuration and then
pass a new flag value interactively to override it. However, `jj` does not have
arbitrary "negation" options such as `--no-interactive`, so there are some cases
where this approach doesn't work. (As expected, options that accept multiple
values, such as `--config`, will see all given values, not only the last one.)

Since this decision directly affects the way users interact with the CLI, there
may of course be exceptions to this. For example,
[`ui.movement.edit`](config.md#behavior-of-prev-and-next-commands) controls
whether `jj prev` and `jj next` directly edit the target revision or create a
new child on top of it. If consensus can be reached about a config option
modifying default behavior in this way, the project will accept it. Please
propose and discuss an idea if you have one before implementing it.

References: [#1509](https://github.com/jj-vcs/jj/issues/1509),
[#3947](https://github.com/jj-vcs/jj/issues/3947),
[#4283](https://github.com/jj-vcs/jj/pull/4283)
