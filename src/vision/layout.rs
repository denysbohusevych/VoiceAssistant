use serde::Deserialize;
use std::collections::VecDeque;

// ═══════════════════════════════════════════════════════════════════════════════
// АРХИТЕКТУРА: 3 слоя
//
//  [Слой 1] Геометрия      — merge_words_into_lines, cluster_lines_into_blocks,
//                            split_into_columns, merge_small_columns
//                            Работает только с координатами. Никакой семантики.
//                            Универсален для любого приложения.
//
//  [Слой 2] Нейтральный XML — generate_layout_xml
//                            Выводит факты: координаты, размеры, текст.
//                            Никаких ролей, никаких предположений о типе UI.
//
//  [Слой 3] Классификация  — НЕ реализован здесь намеренно.
//
//            Проблема: если делать конкретные классификаторы (MessengerClassifier,
//            DocumentClassifier, IDEClassifier...) — их будет бесконечно много,
//            и каждый будет хрупким.
//
//            Решение: один универсальный классификатор — это LLM.
//            Нейтральный XML из слоя 2 скармливается напрямую в промпт.
//            LLM сама выводит семантику из контекста.
//            Вместо 100 классификаторов — один промпт с инструкцией.
//
//            Если нужна детерминированная классификация для конкретного случая
//            — реализуй трейт `BlockAnnotator` ниже и подключи точечно.
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


// ─── Конфигурация геометрических порогов ─────────────────────────────────────
//
// Все магические числа вынесены сюда.
// Меняй коэффициенты под конкретный тип приложения если дефолты не подходят.

pub struct LayoutConfig {
    /// Порог выравнивания по Y для определения одной строки (в долях median_h)
    pub line_y_alignment_ratio: f64,

    /// Максимальный горизонтальный зазор между словами в одной строке (в долях median_h)
    pub word_merge_x_gap_ratio: f64,

    /// Максимальный отрицательный X-gap (перекрытие слов) допустимый при слиянии
    pub word_merge_x_overlap_ratio: f64,

    /// Максимальный вертикальный зазор между строками в одном блоке (в долях median_h)
    pub block_y_gap_ratio: f64,

    /// Минимальное горизонтальное перекрытие строк для попадания в один блок (в долях median_h)
    pub block_x_overlap_ratio: f64,

    /// Максимальное горизонтальное смещение строк для попадания в один блок
    /// (используется когда нет перекрытия — например выровненные по левому краю абзацы)
    pub block_x_alignment_ratio: f64,

    /// Минимальный зазор между center_x блоков для создания новой колонки (в долях median_h).
    /// Увеличь для сложных интерфейсов типа Slack (5.0–6.0), уменьши для простых (2.0–3.0).
    pub column_gutter_ratio: f64,

    /// Минимальное количество блоков в колонке.
    /// Колонки с меньшим числом блоков сливаются с ближайшим соседом.
    /// Предотвращает мусорные мини-колонки из одиночных timestamps и иконок.
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
    /// Пресет для мессенджеров (Telegram, WhatsApp, iMessage)
    pub fn messenger() -> Self {
        Self {
            column_gutter_ratio: 4.0,
            column_min_blocks:   3,
            ..Default::default()
        }
    }

    /// Пресет для сложных десктопных приложений (Slack, VS Code, Figma)
    pub fn desktop_app() -> Self {
        Self {
            column_gutter_ratio: 6.0,
            column_min_blocks:   2,
            ..Default::default()
        }
    }

    /// Пресет для документов (PDF, Word, браузер)
    pub fn document() -> Self {
        Self {
            column_gutter_ratio:     5.0,
            column_min_blocks:       2,
            block_y_gap_ratio:       1.5, // параграфы могут стоять дальше друг от друга
            block_x_alignment_ratio: 4.0, // текст может иметь отступы
            ..Default::default()
        }
    }
}


// ─── Опциональный трейт для слоя 3 ───────────────────────────────────────────
//
// Если в каком-то конкретном случае нужна детерминированная аннотация —
// реализуй этот трейт и передай в `generate_layout_xml`.
// По умолчанию используется `NoopAnnotator` который ничего не делает.

pub trait BlockAnnotator {
    /// Возвращает дополнительные XML-атрибуты для блока или пустую строку.
    fn annotate_block(&self, block: &LayoutBlock) -> String;

    /// Возвращает дополнительные XML-атрибуты для строки текста или пустую строку.
    fn annotate_line(&self, line: &TextLine) -> String;
}

pub struct NoopAnnotator;

impl BlockAnnotator for NoopAnnotator {
    fn annotate_block(&self, _block: &LayoutBlock) -> String { String::new() }
    fn annotate_line(&self, _line: &TextLine) -> String { String::new() }
}


// ─── Внутренние структуры геометрии ──────────────────────────────────────────

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

    /// Минимальный bbox покрывающий оба прямоугольника
    pub fn union(&self, other: &BoundingBox) -> BoundingBox {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let r = self.right().max(other.right());
        let b = self.bottom().max(other.bottom());
        BoundingBox { x, y, w: r - x, h: b - y }
    }

    /// Горизонтальное перекрытие (положительное = перекрытие, отрицательное = зазор)
    pub fn x_overlap(&self, other: &BoundingBox) -> f64 {
        self.right().min(other.right()) - self.x.max(other.x)
    }

    /// Вертикальный зазор (положительное = зазор, отрицательное = перекрытие)
    pub fn y_gap(&self, other: &BoundingBox) -> f64 {
        self.y.max(other.y) - self.bottom().min(other.bottom())
    }
}

#[derive(Debug, Clone)]
pub struct TextLine {
    pub text: String,
    pub bbox: BoundingBox,
}

/// Публичная структура блока — передаётся в BlockAnnotator
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
    /// Средний center_x всех блоков колонки
    fn center_x(&self) -> f64 {
        if self.blocks.is_empty() { return 0.0; }
        self.blocks.iter().map(|b| b.bbox.center_x()).sum::<f64>() / self.blocks.len() as f64
    }

    /// Общий bbox колонки
    fn bbox(&self) -> Option<BoundingBox> {
        let mut iter = self.blocks.iter();
        let first = iter.next()?.bbox.clone();
        Some(iter.fold(first, |acc, b| acc.union(&b.bbox)))
    }
}


// ─── Слой 1: Геометрия ────────────────────────────────────────────────────────

/// Медиана высот OCR-элементов — базовая единица масштаба.
/// Делает все пороги независимыми от DPI и размера шрифта.
fn compute_median_height(nodes: &[OCRNode]) -> f64 {
    if nodes.is_empty() { return 14.0; }
    let mut heights: Vec<f64> = nodes.iter().map(|n| n.frame.h).collect();
    heights.sort_by(|a, b| a.partial_cmp(b).unwrap());
    heights[heights.len() / 2]
}

/// Слой 1а: Слияние OCR-слов в строки.
///
/// Алгоритм:
/// 1. Сортируем по Y (первично) и X (вторично)
/// 2. Для каждого слова ищем лучшую строку-кандидата по всем строкам
///    (не только по последней — критично при OCR-дрейфе по Y)
/// 3. Из подходящих кандидатов выбираем тот, правый край которого ближе всего
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

        // Ищем лучшего кандидата среди всех строк
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

/// Слой 1б: Кластеризация строк в визуальные блоки (параграфы, пузыри, карточки...).
///
/// Две строки попадают в один блок если:
///   - Вертикальный зазор мал
///   - Горизонтально они "связаны": перекрываются или выровнены по краю
///
/// BFS по графу смежности → связные компоненты = блоки.
/// Никаких предположений о семантике — блок = просто группа близких строк.
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

/// Слой 1в: Разбивка блоков на колонки по кластеризации center_x.
///
/// Почему НЕ sweep-line по правому краю:
///   Sweep-line ломается когда X-диапазоны зон перекрываются.
///   Например в Telegram: сайдбар (x=67–362) и входящие сообщения (x=402–600)
///   — правый край сайдбара почти касается левого края сообщений,
///   и sweep-line объединяет их в одну зону.
///
/// Почему center_x кластеризация работает лучше:
///   center_x сайдбара ≈ 200, входящих ≈ 480, исходящих ≈ 1550.
///   Разрывы в этом распределении чётко соответствуют визуальным зонам
///   независимо от перекрытия X-диапазонов.
fn split_into_columns(blocks: Vec<LayoutBlock>, median_h: f64, cfg: &LayoutConfig) -> Vec<LayoutColumn> {
    if blocks.is_empty() { return Vec::new(); }

    // Собираем и сортируем уникальные center_x
    let mut centers: Vec<f64> = blocks.iter().map(|b| b.bbox.center_x()).collect();
    centers.sort_by(|a, b| a.partial_cmp(b).unwrap());
    centers.dedup_by(|a, b| (*a - *b).abs() < 0.1);

    // Находим границы колонок по разрывам в распределении center_x
    let threshold = median_h * cfg.column_gutter_ratio;
    let mut boundaries: Vec<f64> = vec![f64::MIN];
    for w in centers.windows(2) {
        if w[1] - w[0] > threshold {
            boundaries.push((w[0] + w[1]) / 2.0); // midpoint = граница
        }
    }
    boundaries.push(f64::MAX);

    // Создаём пустые колонки и раскладываем блоки
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

/// Слой 1г: Устранение мусорных микро-колонок.
///
/// Проблема: одиночные timestamps, иконки, стрелки навигации, счётчики реакций
/// создают отдельные колонки с 1–2 блоками. Это происходит везде:
///   - Telegram: даты справа от списка чатов ("Сб", "Пт", "1/03/26")
///   - Slack: стрелки навигации, счётчики реакций, одиночный ">"
///   - Любое приложение: иконки тулбара, элементы статусбара
///
/// Решение: колонки с числом блоков < column_min_blocks сливаются
/// с ближайшим соседом по center_x. Повторяем до сходимости.
fn merge_small_columns(mut columns: Vec<LayoutColumn>, cfg: &LayoutConfig) -> Vec<LayoutColumn> {
    if columns.len() < 2 { return columns; }

    let mut changed = true;
    while changed {
        changed = false;
        let mut i = 0;

        while i < columns.len() {
            if columns.len() < 2 { break; }

            if columns[i].blocks.len() < cfg.column_min_blocks {
                // Ищем ближайшего соседа по center_x
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
                    // После remove индексы сдвинулись — корректируем
                    let target = if nidx > i { nidx - 1 } else { nidx };
                    for block in small.blocks {
                        columns[target].blocks.push(block);
                    }
                    // Пересортируем блоки в целевой колонке по Y
                    columns[target].blocks.sort_by(|a, b| {
                        a.bbox.y.partial_cmp(&b.bbox.y).unwrap()
                    });
                    changed = true;
                    // Не инкрементируем i — проверяем эту же позицию снова
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


// ─── Слой 2: Нейтральный XML ──────────────────────────────────────────────────
//
// XML содержит ТОЛЬКО факты:
//   - Координаты и размеры (x, y, w, h) каждого элемента
//   - Высота шрифта (h) на уровне строки — потребитель сам решает что заголовок
//   - Исходный текст (XML-экранированный)
//
// Намеренно отсутствует: роли (Heading/Body), типы контента, предположения об UI.
// Нейтральность = универсальность. LLM или BlockAnnotator добавят семантику позже.

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Генерирует нейтральный XML-макет экрана.
///
/// `annotator` — опциональный слой 3. Передай `&NoopAnnotator` для чистого
/// геометрического вывода, или свою реализацию `BlockAnnotator` для
/// добавления семантических атрибутов под конкретное приложение.
pub fn generate_layout_xml(
    nodes: Vec<OCRNode>,
    window_title: &str,
    cfg: &LayoutConfig,
    annotator: &dyn BlockAnnotator,
) -> String {
    if nodes.is_empty() {
        return format!(
            "```xml\n<Screen>\n  <Window title=\"{}\" />\n  <!-- No OCR data -->\n</Screen>\n```\n",
            escape_xml(window_title)
        );
    }

    let median_h = compute_median_height(&nodes);
    let lines    = merge_words_into_lines(&nodes, median_h, cfg);
    let blocks   = cluster_lines_into_blocks(&lines, median_h, cfg);
    let columns  = split_into_columns(blocks, median_h, cfg);
    let columns  = merge_small_columns(columns, cfg);

    let mut xml = format!(
        "## Screen layout (OCR):\n```xml\n<Screen median_font_h=\"{}\">\n  <Window title=\"{}\" />\n\n",
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

/// Простой вход: JSON → нейтральный XML с дефолтными настройками.
/// Для большинства случаев достаточно.
pub fn build_layout_from_dump(json_dump: &str) -> String {
    build_layout_from_dump_with(json_dump, &LayoutConfig::default(), &NoopAnnotator)
}

/// Расширенный вход: кастомный конфиг и аннотатор.
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

    generate_layout_xml(dump.ocr_text, &title, cfg, annotator)
}


// ─── Пример: Слой 3 для мессенджеров ─────────────────────────────────────────
//
// Раскомментируй если нужна детерминированная аннотация сторон диалога.
// В большинстве случаев лучше скормить нейтральный XML в LLM.
//
// pub struct MessengerAnnotator {
//     /// X-координата середины чат-панели.
//     /// Блоки левее → incoming, правее → outgoing.
//     pub chat_pane_center_x: f64,
// }
//
// impl BlockAnnotator for MessengerAnnotator {
//     fn annotate_block(&self, block: &LayoutBlock) -> String {
//         let side = if block.bbox.center_x() < self.chat_pane_center_x {
//             "incoming"
//         } else {
//             "outgoing"
//         };
//         format!("side=\"{}\"", side)
//     }
//     fn annotate_line(&self, _line: &TextLine) -> String { String::new() }
// }