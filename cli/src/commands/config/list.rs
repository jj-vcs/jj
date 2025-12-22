// Copyright 2020 The Jujutsu Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// https://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use clap_complete::ArgValueCandidates;
use jj_lib::config::ConfigNamePathBuf;
use jj_lib::config::ConfigSource;
use jj_lib::settings::UserSettings;
use tracing::instrument;

use super::ConfigLevelArgs;
use crate::cli_util::CommandHelper;
use crate::command_error::CommandError;
use crate::complete;
use crate::config::AnnotatedValue;
use crate::config::resolved_config_values;
use crate::generic_templater;
use crate::generic_templater::GenericTemplateLanguage;
use crate::templater::TemplatePropertyExt as _;
use crate::templater::TemplateRenderer;
use crate::ui::Ui;

/// List variables set in config files, along with their values.
#[derive(clap::Args, Clone, Debug)]
#[command(mut_group("config_level", |g| g.required(false)))]
pub struct ConfigListArgs {
    /// An optional name of a specific config option to look up.
    #[arg(add = ArgValueCandidates::new(complete::config_keys))]
    pub name: Option<ConfigNamePathBuf>,
    /// Whether to explicitly include built-in default values in the list.
    #[arg(long, conflicts_with = "config_level")]
    pub include_defaults: bool,
    /// Allow printing overridden values.
    #[arg(long)]
    pub include_overridden: bool,
    #[command(flatten)]
    pub level: ConfigLevelArgs,
    /// Render each variable using the given template
    ///
    /// The following keywords are available in the template expression:
    ///
    /// * `name: String`: Config name, in [TOML's "dotted key" format].
    /// * `value: ConfigValue`: Value to be formatted in TOML syntax.
    /// * `overridden: Boolean`: True if the value is shadowed by other.
    /// * `source: String`: Source of the value.
    /// * `path: String`: Path to the config file.
    ///
    /// Can be overridden by the `templates.config_list` setting. To
    /// see a detailed config list, use the `builtin_config_list_detailed`
    /// template.
    ///
    /// See [`jj help -k templates`] for more information.
    ///
    /// [TOML's "dotted key" format]: https://toml.io/en/v1.0.0#keys
    ///
    /// [`jj help -k templates`]:
    ///     https://docs.jj-vcs.dev/latest/templates/
    #[arg(
        long, short = 'T',
        verbatim_doc_comment,
        add = ArgValueCandidates::new(complete::template_aliases)
    )]
    template: Option<String>,
}

#[instrument(skip_all)]
pub fn cmd_config_list(
    ui: &mut Ui,
    command: &CommandHelper,
    args: &ConfigListArgs,
) -> Result<(), CommandError> {
    let template: TemplateRenderer<AnnotatedValue> = {
        let language = config_template_language(command.settings());
        let text = match &args.template {
            Some(value) => value.to_owned(),
            None => command.settings().get_string("templates.config_list")?,
        };
        command
            .parse_template(ui, &language, &text)?
            .labeled(["config_list"])
    };

    let name_path = args.name.clone().unwrap_or_else(ConfigNamePathBuf::root);
    let mut annotated_values = resolved_config_values(command.settings().config(), &name_path);
    // The default layer could be excluded beforehand as layers[len..], but we
    // can't do the same for "annotated.source == target_source" in order for
    // resolved_config_values() to mark values overridden by the upper layers.
    if let Some(target_source) = args.level.get_source_kind() {
        annotated_values.retain(|annotated| annotated.source == target_source);
    } else if !args.include_defaults {
        annotated_values.retain(|annotated| annotated.source != ConfigSource::Default);
    }
    if !args.include_overridden {
        annotated_values.retain(|annotated| !annotated.is_overridden);
    }

    if !annotated_values.is_empty() {
        ui.request_pager();
        let mut formatter = ui.stdout_formatter();
        for annotated in &annotated_values {
            template.format(annotated, formatter.as_mut())?;
        }
    } else {
        // Note to stderr explaining why output is empty.
        if let Some(name) = &args.name {
            writeln!(ui.warning_default(), "No matching config key for {name}")?;
        } else {
            writeln!(ui.warning_default(), "No config to list")?;
        }
    }
    Ok(())
}

type ConfigTemplateLanguage = GenericTemplateLanguage<'static, AnnotatedValue>;

generic_templater::impl_self_property_wrapper!(AnnotatedValue);

// AnnotatedValue will be cloned internally in the templater. If the cloning
// cost matters, wrap it with Rc.
fn config_template_language(settings: &UserSettings) -> ConfigTemplateLanguage {
    let mut language = ConfigTemplateLanguage::new(settings);
    language.add_keyword("name", |self_property| {
        let out_property = self_property.map(|annotated| annotated.name.to_string());
        Ok(out_property.into_dyn_wrapped())
    });
    language.add_keyword("value", |self_property| {
        // .decorated("", "") to trim leading/trailing whitespace
        let out_property = self_property.map(|annotated| annotated.value.decorated("", ""));
        Ok(out_property.into_dyn_wrapped())
    });
    language.add_keyword("source", |self_property| {
        let out_property = self_property.map(|annotated| annotated.source.to_string());
        Ok(out_property.into_dyn_wrapped())
    });
    language.add_keyword("path", |self_property| {
        let out_property = self_property.map(|annotated| {
            // TODO: maybe add FilePath(PathBuf) template type?
            annotated
                .path
                .as_ref()
                .map_or_else(String::new, |path| path.to_string_lossy().into_owned())
        });
        Ok(out_property.into_dyn_wrapped())
    });
    language.add_keyword("overridden", |self_property| {
        let out_property = self_property.map(|annotated| annotated.is_overridden);
        Ok(out_property.into_dyn_wrapped())
    });
    language
}
