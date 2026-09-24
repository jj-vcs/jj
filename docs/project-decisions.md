# Project Decisions

This is a list of some decisions that the project has made in the past,
especially regarding certain features. If you would like to revisit these
decisions, feel free to start a discussion on GitHub or Discord, but be prepared
for maintainers to still feel the same way. Do not send a PR implementing one of
these features as a "proposal"; we will probably just link you to this page.

### AI/LLM Policy

Please see the [AI policy].

[AI policy]:
  https://docs.jj-vcs.dev/latest/contributing/#use-of-ai-and-llm-tools

References: [#9219](https://github.com/jj-vcs/jj/issues/9219),
[#10057](https://github.com/jj-vcs/jj/pull/10057)

### Community aliases / configs / shareables

The project avoids to be the source of truth / hosting point for community
shareables (such as aliases, configs, agent configurations, etc.) since those
are community responsibilities which are not relevant to the development of the
project. Please see the [GitHub Wiki] or [Community Tools] page for existing
community efforts. In the future we may prefer to use a trusted community
organization or website.

References:
[#8780](https://github.com/jj-vcs/jj/issues/8780#issuecomment-3842154330),
[Discord discussion](https://discord.com/channels/968932220549103686/968932220549103689/1441126185512992962)

[GitHub Wiki]: https://github.com/jj-vcs/jj/wiki
[Community Tools]: https://docs.jj-vcs.dev/latest/community_tools/

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
with deprecating it, are (quoted from @martinvonz in [#1509][1509 comment]):

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
new child on top of it. However, these are really special cases, and must
demonstrate real needs from a large number of users for the project to consider
them.

References: [#1509](https://github.com/jj-vcs/jj/issues/1509),
[#3947](https://github.com/jj-vcs/jj/issues/3947),
[#4283](https://github.com/jj-vcs/jj/pull/4283)

[1509 comment]: https://github.com/jj-vcs/jj/issues/1509#issuecomment-1504684964

### Command names

The project does its best to define CLI subcommand names as English verbs, such
that `jj <action>` makes sense for what the user is trying to do (e.g.,
`jj new`, `jj edit`, `jj bookmark create`) and is ideally not a made-up or super
domain-specific word. We also usually prefer additional subcommands (as
appropriate) rather than hyphenated or compound words. There have been
historical deviations from these guidelines (e.g., `metaedit` and
`simplify-parents`), and they're not hard rules, but we do try to make the CLI
surface nice to use.

Furthermore, `jj` is its own CLI; it does not aim to be a copy of other VCS
CLIs. For example, we have `jj arrange` instead of `hg histedit`, and in general
`jj` commands do not nudge themselves to be closer to names familiar to
migrating users, such as `checkout`, `backout`, `pull`, etc. (One exception to
this is [`jj commit`][commit cmd], which can be equivalent to combinations of
existing commands but is provided as a convenience.) We of course take
inspiration and suggestions from VCS predecessors, but `jj` will not copy them
directly if a better alternative exists. We are happy to provide command tables
for common VCS operations (currently only for Git, but other VCS command tables
can be added if requested).

References: [#7389](https://github.com/jj-vcs/jj/issues/7389)

[commit cmd]: https://docs.jj-vcs.dev/latest/cli-reference/#jj-commit
