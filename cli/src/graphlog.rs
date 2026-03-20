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

use std::collections::HashSet;
use std::hash::Hash;
use std::io;
use std::io::Write;

use jj_lib::config::ConfigGetError;
use jj_lib::graph::GraphEdge;
use jj_lib::graph::GraphEdgeType;
use jj_lib::settings::UserSettings;
use renderdag::Ancestor;
use renderdag::GraphRowRenderer;
use renderdag::Renderer;

pub trait GraphLog<K: Clone + Eq + Hash> {
    fn add_node(
        &mut self,
        id: &K,
        edges: &[GraphEdge<K>],
        node_symbol: &str,
        text: &str,
    ) -> io::Result<()>;

    fn width(&self, id: &K, edges: &[GraphEdge<K>]) -> usize;
}

pub struct SaplingGraphLog<'writer, K, R> {
    renderer: R,
    writer: &'writer mut dyn Write,
    /// IDs we expect to see as future nodes (targets of edges from
    /// already-emitted nodes). Used for component boundary detection.
    expected_ids: HashSet<K>,
    /// Whether we have emitted at least one node.
    has_emitted: bool,
}

fn convert_graph_edge_into_ancestor<K: Clone>(e: &GraphEdge<K>) -> Ancestor<K> {
    match e.edge_type {
        GraphEdgeType::Direct => Ancestor::Parent(e.target.clone()),
        GraphEdgeType::Indirect => Ancestor::Ancestor(e.target.clone()),
        GraphEdgeType::Missing => Ancestor::Anonymous,
    }
}

impl<K, R> GraphLog<K> for SaplingGraphLog<'_, K, R>
where
    K: Clone + Eq + Hash,
    R: Renderer<K, Output = String>,
{
    fn add_node(
        &mut self,
        id: &K,
        edges: &[GraphEdge<K>],
        node_symbol: &str,
        text: &str,
    ) -> io::Result<()> {
        // Detect component boundaries: if all pending edges from previous
        // nodes have been drained, this node starts a new disconnected
        // component. Insert a blank line to visually separate them.
        if self.has_emitted && self.expected_ids.is_empty() {
            writeln!(self.writer)?;
        }
        self.has_emitted = true;
        self.expected_ids.remove(id);

        // Track edge targets as expected future nodes.
        for edge in edges {
            self.expected_ids.insert(edge.target.clone());
        }

        let row = self.renderer.next_row(
            id.clone(),
            edges.iter().map(convert_graph_edge_into_ancestor).collect(),
            node_symbol.into(),
            text.into(),
        );

        write!(self.writer, "{row}")
    }

    fn width(&self, id: &K, edges: &[GraphEdge<K>]) -> usize {
        let parents = edges.iter().map(convert_graph_edge_into_ancestor).collect();
        let w: u64 = self.renderer.width(Some(id), Some(&parents));
        w.try_into().unwrap()
    }
}

impl<'writer, K, R> SaplingGraphLog<'writer, K, R> {
    pub fn create(renderer: R, formatter: &'writer mut dyn Write) -> Box<dyn GraphLog<K> + 'writer>
    where
        K: Clone + Eq + Hash + 'writer,
        R: Renderer<K, Output = String> + 'writer,
    {
        Box::new(SaplingGraphLog {
            renderer,
            writer: formatter,
            expected_ids: HashSet::new(),
            has_emitted: false,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize)]
#[serde(rename_all(deserialize = "kebab-case"))]
pub enum GraphStyle {
    Ascii,
    AsciiLarge,
    Curved,
    Square,
}

impl GraphStyle {
    pub fn from_settings(settings: &UserSettings) -> Result<Self, ConfigGetError> {
        settings.get("ui.graph.style")
    }
}

pub fn get_graphlog<'a, K: Clone + Eq + Hash + 'a>(
    style: GraphStyle,
    formatter: &'a mut dyn Write,
) -> Box<dyn GraphLog<K> + 'a> {
    let builder = GraphRowRenderer::new().output().with_min_row_height(0);
    match style {
        GraphStyle::Ascii => SaplingGraphLog::create(builder.build_ascii(), formatter),
        GraphStyle::AsciiLarge => SaplingGraphLog::create(builder.build_ascii_large(), formatter),
        GraphStyle::Curved => SaplingGraphLog::create(builder.build_box_drawing(), formatter),
        GraphStyle::Square => {
            SaplingGraphLog::create(builder.build_box_drawing().with_square_glyphs(), formatter)
        }
    }
}
