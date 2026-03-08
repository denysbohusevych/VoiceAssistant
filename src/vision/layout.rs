use serde::Deserialize;
use std::collections::VecDeque;

// ═══════════════════════════════════════════════════════════════════════════════
// АРХИТЕКТУРА: 3 слоя
//
//  [Слой 1] Геометрия      — Кластеризация и геометрический анализ (OCR Фолбек)
//  [Слой 2] Нейтральный XML — Генерация XML-макета. Теперь с двумя стратегиями:
//                            А) AX Tree (основная) - рекурсивный обход дерева.
//                            Б) OCR (фолбек) - если дерево "пустое" (напр. Electron).
//  [Слой 3] Классификация  — LLM на основе нейтрального XML.
// ═══════════════════════════════════════════════════════════════════════════════

// ─── Входные данные (JSON от ax-helper) ──────────────────────────────────────

#[derive(Debug, Deserialize, Clone)]
pub struct FrameData {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

#[derive(Debug, Deserialize)]
pub struct AXNode {
    pub role: String,
    pub title: Option<String>,
    pub value: Option<String>,
    pub description: Option<String>,
    pub frame: Option<FrameData>,
    pub children: Option<Vec<AXNode>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OCRNode {
    pub text: String,
    pub frame: FrameData,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DumpOutput {
    pub ax_tree: Option<AXNode>,
    pub ocr_text: Vec<OCRNode>,
}

// ─── Конфигурация геометрических порогов (для OCR Фолбека) ───────────────────

pub struct LayoutConfig {
    pub line_y_alignment_ratio: f64,
    pub word_merge_x_gap_ratio: f64,
    pub word_merge_x_overlap_ratio: f64,
    pub block_y_gap_ratio: f64,
    pub block_x_overlap_ratio: f64,
    pub block_x_alignment_ratio: f64,
    pub column_gutter_ratio: f64,
    pub column_min_blocks: usize,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            line_y_alignment_ratio:     0.4,
            word_merge_x_gap_ratio:     1.5,
            word_merge_x_overlap_ratio: 1.0,
            block_y_gap_ratio:          1.2,
            block_x_overlap_ratio:      0.5,
            block_x_alignment_ratio:    3.0,
            column_gutter_ratio:        4.0,
            column_min_blocks:          3,
        }
    }
}

impl LayoutConfig {
    pub fn messenger() -> Self {
        Self {
            column_gutter_ratio: 4.0,
            column_min_blocks:   3,
            ..Default::default()
        }
    }

    pub fn desktop_app() -> Self {
        Self {
            column_gutter_ratio: 6.0,
            column_min_blocks:   2,
            ..Default::default()
        }
    }

    pub fn document() -> Self {
        Self {
            column_gutter_ratio:     5.0,
            column_min_blocks:       2,
            block_y_gap_ratio:       1.5,
            block_x_alignment_ratio: 4.0,
            ..Default::default()
        }
    }
}

pub trait BlockAnnotator {
    fn annotate_block(&self, block: &LayoutBlock) -> String;
    fn annotate_line(&self, line: &TextLine) -> String;
}

pub struct NoopAnnotator;

impl BlockAnnotator for NoopAnnotator {
    fn annotate_block(&self, _block: &LayoutBlock) -> String { String::new() }
    fn annotate_line(&self, _line: &TextLine) -> String { String::new() }
}

// ─── Внутренние структуры геометрии (для OCR) ────────────────────────────────

#[derive(Debug, Clone)]
pub struct BoundingBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl BoundingBox {
    pub fn right(&self)    -> f64 { self.x + self.w }
    pub fn bottom(&self)   -> f64 { self.y + self.h }
    pub fn center_x(&self) -> f64 { self.x + self.w / 2.0 }
    pub fn center_y(&self) -> f64 { self.y + self.h / 2.0 }

    pub fn union(&self, other: &BoundingBox) -> BoundingBox {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let r = self.right().max(other.right());
        let b = self.bottom().max(other.bottom());
        BoundingBox { x, y, w: r - x, h: b - y }
    }

    pub fn x_overlap(&self, other: &BoundingBox) -> f64 {
        self.right().min(other.right()) - self.x.max(other.x)
    }

    pub fn y_gap(&self, other: &BoundingBox) -> f64 {
        self.y.max(other.y) - self.bottom().min(other.bottom())
    }
}

#[derive(Debug, Clone)]
pub struct TextLine {
    pub text: String,
    pub bbox: BoundingBox,
}

#[derive(Debug, Clone)]
pub struct LayoutBlock {
    pub bbox:  BoundingBox,
    pub lines: Vec<TextLine>,
}

#[derive(Debug, Clone)]
struct LayoutColumn {
    blocks: Vec<LayoutBlock>,
}

impl LayoutColumn {
    fn center_x(&self) -> f64 {
        if self.blocks.is_empty() { return 0.0; }
        self.blocks.iter().map(|b| b.bbox.center_x()).sum::<f64>() / self.blocks.len() as f64
    }

    fn bbox(&self) -> Option<BoundingBox> {
        let mut iter = self.blocks.iter();
        let first = iter.next()?.bbox.clone();
        Some(iter.fold(first, |acc, b| acc.union(&b.bbox)))
    }
}

// ─── Слой 1: Геометрия (OCR Фолбек алгоритмы) ─────────────────────────────────

fn compute_median_height(nodes: &[OCRNode]) -> f64 {
    if nodes.is_empty() { return 14.0; }
    let mut heights: Vec<f64> = nodes.iter().map(|n| n.frame.h).collect();
    heights.sort_by(|a, b| a.partial_cmp(b).unwrap());
    heights[heights.len() / 2]
}

fn merge_words_into_lines(nodes: &[OCRNode], median_h: f64, cfg: &LayoutConfig) -> Vec<TextLine> {
    let mut sorted = nodes.to_vec();
    sorted.sort_by(|a, b| {
        let y_diff = (a.frame.y - b.frame.y).abs();
        if y_diff < median_h * cfg.line_y_alignment_ratio {
            a.frame.x.partial_cmp(&b.frame.x).unwrap()
        } else {
            a.frame.y.partial_cmp(&b.frame.y).unwrap()
        }
    });

    let mut lines: Vec<TextLine> = Vec::new();

    for el in &sorted {
        let el_bbox = BoundingBox {
            x: el.frame.x, y: el.frame.y,
            w: el.frame.w, h: el.frame.h,
        };
        let el_center_y = el_bbox.center_y();

        let best_idx = lines.iter().enumerate()
            .filter_map(|(i, line)| {
                let y_diff = (line.bbox.center_y() - el_center_y).abs();
                let x_gap  = el_bbox.x - line.bbox.right();

                let y_ok = y_diff < median_h * cfg.line_y_alignment_ratio;
                let x_ok = x_gap >= -(median_h * cfg.word_merge_x_overlap_ratio)
                    && x_gap <   (median_h * cfg.word_merge_x_gap_ratio);

                if y_ok && x_ok { Some((i, x_gap)) } else { None }
            })
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .map(|(i, _)| i);

        if let Some(idx) = best_idx {
            let line  = &mut lines[idx];
            let x_gap = el_bbox.x - line.bbox.right();
            if x_gap > 0.5 { line.text.push(' '); }
            line.text.push_str(&el.text);
            line.bbox = line.bbox.union(&el_bbox);
        } else {
            lines.push(TextLine { text: el.text.clone(), bbox: el_bbox });
        }
    }

    lines
}

fn cluster_lines_into_blocks(lines: &[TextLine], median_h: f64, cfg: &LayoutConfig) -> Vec<LayoutBlock> {
    let n = lines.len();
    if n == 0 { return Vec::new(); }

    let mut adj: Vec<Vec<usize>> = vec![vec![]; n];

    for i in 0..n {
        for j in (i + 1)..n {
            let a = &lines[i].bbox;
            let b = &lines[j].bbox;

            let y_gap     = a.y_gap(b);
            let x_overlap = a.x_overlap(b);

            let vertically_close    = y_gap < median_h * cfg.block_y_gap_ratio;
            let horizontally_linked =
                x_overlap > median_h * cfg.block_x_overlap_ratio
                    || (a.x - b.x).abs() < median_h * cfg.block_x_alignment_ratio;

            if vertically_close && horizontally_linked {
                adj[i].push(j);
                adj[j].push(i);
            }
        }
    }

    let mut visited = vec![false; n];
    let mut blocks  = Vec::new();

    for start in 0..n {
        if visited[start] { continue; }

        let mut component: Vec<TextLine> = Vec::new();
        let mut queue = VecDeque::new();
        visited[start] = true;
        queue.push_back(start);

        while let Some(curr) = queue.pop_front() {
            component.push(lines[curr].clone());
            for &neighbor in &adj[curr] {
                if !visited[neighbor] {
                    visited[neighbor] = true;
                    queue.push_back(neighbor);
                }
            }
        }

        component.sort_by(|a, b| a.bbox.y.partial_cmp(&b.bbox.y).unwrap());

        let bbox = component.iter().skip(1).fold(
            component[0].bbox.clone(),
            |acc, l| acc.union(&l.bbox),
        );

        blocks.push(LayoutBlock { bbox, lines: component });
    }

    blocks
}

fn split_into_columns(blocks: Vec<LayoutBlock>, median_h: f64, cfg: &LayoutConfig) -> Vec<LayoutColumn> {
    if blocks.is_empty() { return Vec::new(); }

    let mut centers: Vec<f64> = blocks.iter().map(|b| b.bbox.center_x()).collect();
    centers.sort_by(|a, b| a.partial_cmp(b).unwrap());
    centers.dedup_by(|a, b| (*a - *b).abs() < 0.1);

    let threshold = median_h * cfg.column_gutter_ratio;
    let mut boundaries: Vec<f64> = vec![f64::MIN];
    for w in centers.windows(2) {
        if w[1] - w[0] > threshold {
            boundaries.push((w[0] + w[1]) / 2.0);
        }
    }
    boundaries.push(f64::MAX);

    let num_cols = boundaries.len() - 1;
    let mut columns: Vec<LayoutColumn> = (0..num_cols)
        .map(|_| LayoutColumn { blocks: Vec::new() })
        .collect();

    for block in blocks {
        let cx = block.bbox.center_x();
        let col_idx = boundaries.windows(2)
            .position(|w| cx > w[0] && cx <= w[1])
            .unwrap_or(num_cols - 1);
        columns[col_idx].blocks.push(block);
    }

    columns.retain(|c| !c.blocks.is_empty());
    columns
}

fn merge_small_columns(mut columns: Vec<LayoutColumn>, cfg: &LayoutConfig) -> Vec<LayoutColumn> {
    if columns.len() < 2 { return columns; }

    let mut changed = true;
    while changed {
        changed = false;
        let mut i = 0;

        while i < columns.len() {
            if columns.len() < 2 { break; }

            if columns[i].blocks.len() < cfg.column_min_blocks {
                let cx = columns[i].center_x();

                let neighbor_idx = (0..columns.len())
                    .filter(|&j| j != i)
                    .min_by(|&a, &b| {
                        let da = (columns[a].center_x() - cx).abs();
                        let db = (columns[b].center_x() - cx).abs();
                        da.partial_cmp(&db).unwrap()
                    });

                if let Some(nidx) = neighbor_idx {
                    let small = columns.remove(i);
                    let target = if nidx > i { nidx - 1 } else { nidx };
                    for block in small.blocks {
                        columns[target].blocks.push(block);
                    }
                    columns[target].blocks.sort_by(|a, b| {
                        a.bbox.y.partial_cmp(&b.bbox.y).unwrap()
                    });
                    changed = true;
                } else {
                    i += 1;
                }
            } else {
                i += 1;
            }
        }
    }

    columns
}

// ─── Утилиты ──────────────────────────────────────────────────────────────────

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ─── Стратегия А: Генерация XML по AX-дереву (Основная) ───────────────────────

/// Проверяет, является ли роль интерактивной (даже если пока без текста)
fn is_interactive_role(role: &str) -> bool {
    matches!(
        role,
        "AXTextField" | "AXTextArea" | "AXComboBox" | "AXCheckBox" |
        "AXRadioButton" | "AXButton" | "AXSlider" | "AXPopUpButton" |
        "AXLink" | "AXSearchField"
    )
}

/// Проверяет, есть ли смысл добавлять узел в XML
fn is_node_useful(node: &AXNode) -> bool {
    if is_interactive_role(&node.role) { return true; }
    if node.title.as_ref().map_or(false, |s| !s.trim().is_empty()) { return true; }
    if node.value.as_ref().map_or(false, |s| !s.trim().is_empty()) { return true; }
    if node.description.as_ref().map_or(false, |s| !s.trim().is_empty()) { return true; }
    if let Some(children) = &node.children {
        for c in children {
            if is_node_useful(c) { return true; }
        }
    }
    false
}

/// Подсчет полезных узлов (чтобы понять, работает ли AX Tree в приложении)
fn count_useful_nodes(node: &AXNode) -> usize {
    let mut count = 0;

    let has_content = node.title.as_ref().map_or(false, |s| !s.trim().is_empty()) ||
        node.value.as_ref().map_or(false, |s| !s.trim().is_empty()) ||
        node.description.as_ref().map_or(false, |s| !s.trim().is_empty());

    if has_content || is_interactive_role(&node.role) {
        count += 1;
    }

    if let Some(children) = &node.children {
        for child in children {
            count += count_useful_nodes(child);
        }
    }
    count
}

/// Вычисляет BoundingBox, который охватывает только РЕАЛЬНЫЕ элементы контента
/// (игнорируя прозрачные окна и группы-контейнеры).
fn get_useful_content_bounds(node: &AXNode) -> Option<BoundingBox> {
    let mut bounds: Option<BoundingBox> = None;

    // Игнорируем структурные контейнеры, так как их рамка может занимать 100% экрана,
    // даже если внутри них пусто (как в случае с Chrome/Electron).
    let is_structural = matches!(
        node.role.as_str(),
        "AXWindow" | "AXGroup" | "AXScrollArea" | "AXSplitGroup" | "AXTabGroup" | "AXToolbar" | "AXApplication"
    );

    if !is_structural {
        let has_content = node.title.as_ref().map_or(false, |s| !s.trim().is_empty()) ||
            node.value.as_ref().map_or(false, |s| !s.trim().is_empty()) ||
            node.description.as_ref().map_or(false, |s| !s.trim().is_empty());

        if has_content || is_interactive_role(&node.role) {
            if let Some(f) = &node.frame {
                bounds = Some(BoundingBox { x: f.x, y: f.y, w: f.w, h: f.h });
            }
        }
    }

    if let Some(children) = &node.children {
        for child in children {
            if let Some(child_bounds) = get_useful_content_bounds(child) {
                bounds = match bounds {
                    Some(b) => Some(b.union(&child_bounds)),
                    None => Some(child_bounds),
                };
            }
        }
    }

    bounds
}

fn serialize_ax_tree_recursive(node: &AXNode, indent: usize, xml: &mut String) {
    if !is_node_useful(node) {
        return; // Пропускаем пустые группы/контейнеры для экономии токенов
    }

    let ind = " ".repeat(indent);
    xml.push_str(&ind);
    xml.push_str(&format!("<Node role=\"{}\"", escape_xml(&node.role)));

    if let Some(t) = &node.title {
        if !t.trim().is_empty() { xml.push_str(&format!(" title=\"{}\"", escape_xml(t))); }
    }
    if let Some(v) = &node.value {
        if !v.trim().is_empty() { xml.push_str(&format!(" value=\"{}\"", escape_xml(v))); }
    }
    if let Some(d) = &node.description {
        if !d.trim().is_empty() { xml.push_str(&format!(" description=\"{}\"", escape_xml(d))); }
    }
    if let Some(f) = &node.frame {
        xml.push_str(&format!(" x=\"{}\" y=\"{}\" w=\"{}\" h=\"{}\"", f.x as i32, f.y as i32, f.w as i32, f.h as i32));
    }

    if let Some(children) = &node.children {
        let useful_children: Vec<_> = children.iter().filter(|c| is_node_useful(c)).collect();
        if !useful_children.is_empty() {
            xml.push_str(">\n");
            for child in useful_children {
                serialize_ax_tree_recursive(child, indent + 2, xml);
            }
            xml.push_str(&ind);
            xml.push_str("</Node>\n");
        } else {
            xml.push_str(" />\n");
        }
    } else {
        xml.push_str(" />\n");
    }
}

pub fn generate_ax_layout_xml(root: &AXNode, window_title: &str) -> String {
    let mut xml = format!(
        "## Screen layout (AX Tree):\n```xml\n<Screen source=\"ax\">\n  <Window title=\"{}\" />\n\n",
        escape_xml(window_title)
    );
    serialize_ax_tree_recursive(root, 2, &mut xml);
    xml.push_str("</Screen>\n```\n");
    xml
}

// ─── Стратегия Б: Генерация XML по OCR (Фолбек) ───────────────────────────────

pub fn generate_ocr_layout_xml(
    nodes: Vec<OCRNode>,
    window_title: &str,
    cfg: &LayoutConfig,
    annotator: &dyn BlockAnnotator,
) -> String {
    if nodes.is_empty() {
        return format!(
            "```xml\n<Screen source=\"ocr\">\n  <Window title=\"{}\" />\n  <!-- No OCR data -->\n</Screen>\n```\n",
            escape_xml(window_title)
        );
    }

    let median_h = compute_median_height(&nodes);
    let lines    = merge_words_into_lines(&nodes, median_h, cfg);
    let blocks   = cluster_lines_into_blocks(&lines, median_h, cfg);
    let columns  = split_into_columns(blocks, median_h, cfg);
    let columns  = merge_small_columns(columns, cfg);

    let mut xml = format!(
        "## Screen layout (OCR Fallback):\n```xml\n<Screen source=\"ocr\" median_font_h=\"{}\">\n  <Window title=\"{}\" />\n\n",
        median_h as i32,
        escape_xml(window_title),
    );

    for (col_idx, col) in columns.iter().enumerate() {
        let col_bbox = match col.bbox() {
            Some(b) => b,
            None    => continue,
        };

        xml.push_str(&format!(
            "  <Column id=\"{}\" x=\"{}\" y=\"{}\" w=\"{}\" h=\"{}\">\n",
            col_idx + 1,
            col_bbox.x as i32, col_bbox.y as i32,
            col_bbox.w as i32, col_bbox.h as i32,
        ));

        let mut sorted_blocks = col.blocks.clone();
        sorted_blocks.sort_by(|a, b| a.bbox.y.partial_cmp(&b.bbox.y).unwrap());

        for block in &sorted_blocks {
            let extra_block = annotator.annotate_block(block);
            let block_attrs = if extra_block.is_empty() {
                String::new()
            } else {
                format!(" {}", extra_block)
            };

            xml.push_str(&format!(
                "    <Block x=\"{}\" y=\"{}\" w=\"{}\" h=\"{}\"{}>\n",
                block.bbox.x as i32, block.bbox.y as i32,
                block.bbox.w as i32, block.bbox.h as i32,
                block_attrs,
            ));

            for line in &block.lines {
                let extra_line = annotator.annotate_line(line);
                let line_attrs = if extra_line.is_empty() {
                    String::new()
                } else {
                    format!(" {}", extra_line)
                };

                xml.push_str(&format!(
                    "      <Line x=\"{}\" y=\"{}\" h=\"{}\"{}>{}</Line>\n",
                    line.bbox.x as i32,
                    line.bbox.y as i32,
                    line.bbox.h as i32,
                    line_attrs,
                    escape_xml(&line.text),
                ));
            }

            xml.push_str("    </Block>\n");
        }

        xml.push_str("  </Column>\n\n");
    }

    xml.push_str("</Screen>\n```\n");
    xml
}

// ─── Публичный API ────────────────────────────────────────────────────────────

pub fn build_layout_from_dump(json_dump: &str) -> String {
    build_layout_from_dump_with(json_dump, &LayoutConfig::default(), &NoopAnnotator)
}

pub fn build_layout_from_dump_with(
    json_dump: &str,
    cfg: &LayoutConfig,
    annotator: &dyn BlockAnnotator,
) -> String {
    let dump: DumpOutput = match serde_json::from_str(json_dump) {
        Ok(d)  => d,
        Err(e) => return format!("Parse error: {}", e),
    };

    let title = dump.ax_tree
        .as_ref()
        .and_then(|t| t.title.clone())
        .unwrap_or_else(|| "Unknown".to_string());

    // --- ОЦЕНКА AX ДЕРЕВА И РОУТИНГ ---
    // Если AX-дерево присутствует и богато контентом, используем его.
    if let Some(ax_tree) = &dump.ax_tree {
        let useful_nodes = count_useful_nodes(ax_tree);

        // Получаем общую площадь окна (из корневого фрейма)
        let window_area = ax_tree.frame.as_ref().map(|f| f.w * f.h).unwrap_or(0.0);
        let mut coverage_ok = true;

        if window_area > 0.0 {
            if let Some(content_bounds) = get_useful_content_bounds(ax_tree) {
                let content_area = content_bounds.w * content_bounds.h;
                let coverage_ratio = content_area / window_area;

                // Если реальный интерфейс занимает менее 35% площади окна,
                // значит перед нами "ленивый" рендер (браузер/Electron), где
                // видна только шапка (вкладки/адресная строка), а контент скрыт.
                if coverage_ratio < 0.35 {
                    coverage_ok = false;
                }
            } else {
                coverage_ok = false;
            }
        }

        // Порог: 5 полезных узлов И достаточное покрытие экрана.
        if useful_nodes >= 5 && coverage_ok {
            return generate_ax_layout_xml(ax_tree, &title);
        }
    }

    // В противном случае фолбек на распознавание геометрии через OCR
    generate_ocr_layout_xml(dump.ocr_text, &title, cfg, annotator)
}