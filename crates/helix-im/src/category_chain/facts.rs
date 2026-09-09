//! 分类接龙的 canonical fact 树持久化。
//!
//! 这层只把一份 bounded JSON 树编译成 normalized `StorageOp`。`scope` 本身就是
//! tenant/account-local 的稳定 `snapshot_key`；同一 key 下旧 revision 的尾行保留，
//! 读回时由 head 选中的 revision 再过滤，避免把累计 JSON 或整条 wire 塞进一列。

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::io::{self, Write};

#[cfg(test)]
use helix_core::effect::Row;
use helix_core::effect::{GetSpec, ScanOrder, ScanSpec, SqlValue, StorageOp, UpsertSpec};
use serde_json::{Map, Value};

use crate::error::ImError;

const MAX_DOCUMENT_BYTES: usize = 128 * 1024;
const MAX_FACT_ROWS: usize = 4096;
const MAX_LEAVES: usize = 4096;
const MAX_DEPTH: usize = 16;

/// Host scan 的稳定读取顺序；重建仍会按 pointer depth 再次排序，防止 driver 改变 row 顺序。
const FACT_ORDER: &[ScanOrder] = &[ScanOrder::asc("path")];

#[derive(Debug)]
struct FactRow {
    path: String,
    depth: usize,
    value_json: &'static str,
    owned_value_json: Option<String>,
}

impl FactRow {
    /// 构造一个不需要额外分配的容器 marker 行。
    fn marker(path: String, depth: usize, value_json: &'static str) -> Self {
        Self {
            path,
            depth,
            value_json,
            owned_value_json: None,
        }
    }

    /// 构造一个 scalar 叶行；正文只在叶子列中保存 JSON scalar。
    fn scalar(path: String, depth: usize, value_json: String) -> Self {
        Self {
            path,
            depth,
            value_json: "",
            owned_value_json: Some(value_json),
        }
    }

    /// 返回行中实际要写入的 JSON 文本。
    fn value_json(&self) -> &str {
        self.owned_value_json.as_deref().unwrap_or(self.value_json)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContainerKind {
    Object,
    Array,
}

#[derive(Debug, Default)]
struct FactNode {
    scalar: Option<Value>,
    marker: Option<ContainerKind>,
    children: BTreeMap<String, FactNode>,
}

#[derive(Debug)]
enum ParsedFact {
    Scalar(Value),
    Marker(ContainerKind),
}

/// 把一份 bounded canonical JSON 树编译为 head 行和 normalized fact 行。
///
/// 容器只有在为空或其 key 形状会与数组混淆时才写空 marker；正常非空容器由
/// JSON Pointer 子路径推导类型。这样 4096 个 scalar 叶仍能在固定 4096 行 Scan
/// 界限内读回，同时保留空数组、空对象以及数字对象 key 的精确语义。
pub(crate) fn persist(
    scope: &str,
    revision: &str,
    value: &Value,
) -> Result<Vec<StorageOp>, ImError> {
    validate_revision(revision)?;

    let document_bytes = json_byte_len(value)?;
    if document_bytes > MAX_DOCUMENT_BYTES {
        return Err(ImError::Parse(format!(
            "category fact document exceeds {MAX_DOCUMENT_BYTES} bytes"
        )));
    }

    let mut facts = Vec::new();
    let mut leaves = 0;
    flatten_value(value, String::new(), 0, &mut leaves, &mut facts)?;
    if leaves > MAX_LEAVES {
        return Err(ImError::Parse(format!(
            "category fact document exceeds {MAX_LEAVES} scalar leaves"
        )));
    }
    if facts.len() > MAX_FACT_ROWS {
        return Err(ImError::Parse(format!(
            "category fact document exceeds {MAX_FACT_ROWS} fact rows"
        )));
    }

    facts.sort_unstable_by(|left, right| {
        left.depth
            .cmp(&right.depth)
            .then_with(|| left.path.cmp(&right.path))
    });

    let head = vec![
        ("scope".to_string(), SqlValue::Text(scope.to_owned())),
        ("revision".to_string(), SqlValue::Text(revision.to_owned())),
        ("snapshot_key".to_string(), SqlValue::Text(scope.to_owned())),
    ];
    let fact_rows = facts
        .into_iter()
        .map(|fact| {
            let value_json = fact.value_json().to_owned();
            let path = fact.path;
            vec![
                ("snapshot_key".to_string(), SqlValue::Text(scope.to_owned())),
                (
                    "version_key".to_string(),
                    SqlValue::Text(version_key(scope, revision)),
                ),
                ("path".to_string(), SqlValue::Text(path)),
                ("revision".to_string(), SqlValue::Text(revision.to_owned())),
                ("value_json".to_string(), SqlValue::Text(value_json)),
            ]
        })
        .collect();

    Ok(vec![
        StorageOp::BatchUpsert(UpsertSpec::new(
            "category_chain_head",
            vec![head],
            Some("scope"),
        )),
        StorageOp::BatchUpsert(UpsertSpec::new(
            "category_chain_fact",
            fact_rows,
            Some("snapshot_key,path"),
        )),
    ])
}

/// 生成当前 scope 的 fact Scan；driver 只执行结构化等值过滤和固定上限。
pub(crate) fn read_op(scope: &str, revision: &str) -> StorageOp {
    StorageOp::Scan(ScanSpec {
        table: "category_chain_fact",
        limit: Some(MAX_FACT_ROWS as u32),
        filter: Some(("version_key", SqlValue::Text(version_key(scope, revision)))),
        order_by: FACT_ORDER,
    })
}

/// A canonical decimal suffix unambiguously identifies one revision of a stable scope.
fn version_key(scope: &str, revision: &str) -> String {
    format!("{scope}:{revision}")
}

/// 读取 scope head，用于先确定 durable revision 再做 fact read-back。
pub(crate) fn head_op(scope: &str) -> StorageOp {
    StorageOp::Get(GetSpec {
        table: "category_chain_head",
        key_col: "scope",
        key_val: SqlValue::Text(scope.to_owned()),
    })
}

/// 从 host 的 rows JSON 中按 revision 重建 canonical JSON 树。
///
/// 旧 revision 行在解析字段后立即过滤；marker 只提供容器类型，不会覆盖已经
/// 收集的子节点。任何重复路径、父叶冲突、坏 pointer 或非 scalar 行都会 fail closed。
pub(crate) fn decode(reply: &[u8], revision: &str) -> Result<Value, ImError> {
    validate_revision(revision)?;
    let rows: Value = serde_json::from_slice(reply)
        .map_err(|error| ImError::Parse(format!("category fact rows JSON: {error}")))?;
    let Value::Array(rows) = rows else {
        return Err(ImError::Parse(
            "category fact rows must be a JSON array".to_string(),
        ));
    };
    if rows.len() > MAX_FACT_ROWS {
        return Err(ImError::Parse(format!(
            "category fact reply exceeds {MAX_FACT_ROWS} rows"
        )));
    }

    let mut snapshot_key: Option<String> = None;
    let mut root = FactNode::default();
    let mut selected_rows = 0;
    let mut leaves = 0;

    for row in rows {
        let Value::Object(row) = row else {
            return Err(ImError::Parse(
                "category fact row must be an object".to_string(),
            ));
        };
        let row_snapshot = required_text(&row, "snapshot_key")?;
        if let Some(previous) = snapshot_key.as_deref() {
            if previous != row_snapshot {
                return Err(ImError::Parse(
                    "category fact reply mixes snapshot keys".to_string(),
                ));
            }
        } else {
            snapshot_key = Some(row_snapshot.to_owned());
        }
        let path = required_text(&row, "path")?;
        let row_revision = required_text(&row, "revision")?;
        validate_revision(row_revision)?;
        let segments = decode_pointer(path)?;

        if row_revision != revision {
            continue;
        }

        let value_json = required_text(&row, "value_json")?;
        let value: Value = serde_json::from_str(value_json).map_err(|error| {
            ImError::Parse(format!("category fact value_json at {path:?}: {error}"))
        })?;
        let fact = match value {
            Value::Object(ref object) if object.is_empty() => {
                ParsedFact::Marker(ContainerKind::Object)
            }
            Value::Array(ref array) if array.is_empty() => ParsedFact::Marker(ContainerKind::Array),
            Value::Object(_) | Value::Array(_) => {
                return Err(ImError::Parse(format!(
                    "category fact value at {path:?} must be scalar or empty marker"
                )));
            }
            scalar => {
                leaves += 1;
                if leaves > MAX_LEAVES {
                    return Err(ImError::Parse(format!(
                        "category fact reply exceeds {MAX_LEAVES} scalar leaves"
                    )));
                }
                ParsedFact::Scalar(scalar)
            }
        };
        insert_fact(&mut root, &segments, fact)?;
        selected_rows += 1;
    }

    if selected_rows == 0 {
        return Err(ImError::Parse(format!(
            "category fact revision {revision:?} has no rows"
        )));
    }

    let value = root.build(0)?;
    let encoded = json_byte_len(&value)?;
    if encoded > MAX_DOCUMENT_BYTES {
        return Err(ImError::Parse(format!(
            "decoded category fact exceeds {MAX_DOCUMENT_BYTES} bytes"
        )));
    }
    Ok(value)
}

/// 从 head Get rows 中提取并校验 canonical decimal revision；缺行表示尚未建立 head。
pub(crate) fn head_revision(reply: &[u8]) -> Result<Option<String>, ImError> {
    let rows: Value = serde_json::from_slice(reply)
        .map_err(|error| ImError::Parse(format!("category head rows JSON: {error}")))?;
    let Value::Array(rows) = rows else {
        return Err(ImError::Parse(
            "category head rows must be a JSON array".to_string(),
        ));
    };
    if rows.len() > 1 {
        return Err(ImError::Parse(
            "category head Get returned more than one row".to_string(),
        ));
    }
    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    let Value::Object(row) = row else {
        return Err(ImError::Parse(
            "category head row must be an object".to_string(),
        ));
    };
    let revision = required_text(&row, "revision")?;
    validate_revision(revision)?;
    Ok(Some(revision.to_owned()))
}

/// 严格比较 canonical 十进制 revision，不经过浮点或字典序。
pub(crate) fn revision_cmp(a: &str, b: &str) -> Result<Ordering, ImError> {
    let left = parse_revision(a)?;
    let right = parse_revision(b)?;
    Ok(left.cmp(&right))
}

/// 校验 revision 的 canonical 形状并拒绝超出 u64 的版本。
pub(super) fn validate_revision(revision: &str) -> Result<(), ImError> {
    parse_revision(revision).map(|_| ())
}

/// 解析无前导零、非空且不超过 u64::MAX 的十进制字符串。
fn parse_revision(revision: &str) -> Result<u64, ImError> {
    if revision.is_empty()
        || (revision.len() > 1 && revision.starts_with('0'))
        || !revision.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(ImError::Parse(format!(
            "invalid category revision {revision:?}"
        )));
    }
    revision
        .parse::<u64>()
        .map_err(|_| ImError::Parse(format!("category revision exceeds u64: {revision:?}")))
}

/// 通过 counting writer 计算 canonical JSON 字节数，避免把整份 wire 文档复制到 Vec。
fn json_byte_len(value: &Value) -> Result<usize, ImError> {
    let mut writer = CountingWriter::default();
    serde_json::to_writer(&mut writer, value)
        .map_err(|error| ImError::Serialize(format!("category fact JSON size: {error}")))?;
    Ok(writer.len)
}

#[derive(Default)]
struct CountingWriter {
    len: usize,
}

impl Write for CountingWriter {
    /// 只累计 serializer 输出长度，不保留任何 wire 字节。
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.len = self
            .len
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "JSON size overflow"))?;
        Ok(bytes.len())
    }

    /// counting writer 没有需要 flush 的底层资源。
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// 递归展开 JSON；叶子写 scalar，空/歧义容器写空 marker。
fn flatten_value(
    value: &Value,
    path: String,
    depth: usize,
    leaves: &mut usize,
    facts: &mut Vec<FactRow>,
) -> Result<(), ImError> {
    if depth > MAX_DEPTH {
        return Err(ImError::Parse(format!(
            "category fact nesting exceeds depth {MAX_DEPTH}"
        )));
    }
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            *leaves += 1;
            if *leaves > MAX_LEAVES {
                return Err(ImError::Parse(format!(
                    "category fact document exceeds {MAX_LEAVES} scalar leaves"
                )));
            }
            let value_json = serde_json::to_string(value).map_err(|error| {
                ImError::Serialize(format!("category fact scalar at {path:?}: {error}"))
            })?;
            facts.push(FactRow::scalar(path, depth, value_json));
        }
        Value::Object(object) => {
            let needs_marker = object.is_empty() || object_needs_marker(object);
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort_unstable();
            for key in keys {
                let child = object
                    .get(key)
                    .ok_or_else(|| ImError::Parse("category object key disappeared".to_string()))?;
                flatten_value(child, pointer_child(&path, key), depth + 1, leaves, facts)?;
            }
            if needs_marker {
                facts.push(FactRow::marker(path, depth, "{}"));
            }
        }
        Value::Array(array) => {
            for (index, child) in array.iter().enumerate() {
                flatten_value(
                    child,
                    pointer_child(&path, &index.to_string()),
                    depth + 1,
                    leaves,
                    facts,
                )?;
            }
            if array.is_empty() {
                facts.push(FactRow::marker(path, depth, "[]"));
            }
        }
    }
    Ok(())
}

/// 数字对象 key 若恰好构成数组索引序列，则写 marker 防止读回时被误判为数组。
fn object_needs_marker(object: &Map<String, Value>) -> bool {
    if object.is_empty() {
        return true;
    }
    let mut indexes = object
        .keys()
        .map(|key| canonical_array_index(key))
        .collect::<Option<Vec<_>>>();
    let Some(ref mut indexes) = indexes else {
        return false;
    };
    indexes.sort_unstable();
    indexes
        .iter()
        .enumerate()
        .all(|(expected, actual)| *actual == expected)
}

/// 追加一个 RFC 6901 JSON Pointer segment，并只转义 `~` 与 `/`。
fn pointer_child(parent: &str, segment: &str) -> String {
    let mut path = String::with_capacity(parent.len() + segment.len() + 1);
    path.push_str(parent);
    path.push('/');
    for character in segment.chars() {
        match character {
            '~' => path.push_str("~0"),
            '/' => path.push_str("~1"),
            other => path.push(other),
        }
    }
    path
}

/// 严格解析 RFC 6901 pointer；根路径使用空字符串。
fn decode_pointer(path: &str) -> Result<Vec<String>, ImError> {
    if path.is_empty() {
        return Ok(Vec::new());
    }
    if !path.starts_with('/') {
        return Err(ImError::Parse(format!(
            "category fact path is not JSON Pointer: {path:?}"
        )));
    }
    path[1..].split('/').map(decode_segment).collect()
}

/// 解码单个 pointer segment，并拒绝未定义的 `~` escape。
fn decode_segment(segment: &str) -> Result<String, ImError> {
    let mut decoded = String::with_capacity(segment.len());
    let mut characters = segment.chars();
    while let Some(character) = characters.next() {
        if character != '~' {
            decoded.push(character);
            continue;
        }
        match characters.next() {
            Some('0') => decoded.push('~'),
            Some('1') => decoded.push('/'),
            _ => {
                return Err(ImError::Parse(format!(
                    "category fact path has invalid escape: {segment:?}"
                )));
            }
        }
    }
    Ok(decoded)
}

/// 从 host rows JSON 对象读取必需的字符串列。
fn required_text<'a>(row: &'a Map<String, Value>, column: &str) -> Result<&'a str, ImError> {
    row.get(column)
        .and_then(Value::as_str)
        .ok_or_else(|| ImError::Parse(format!("category fact row missing text column {column}")))
}

/// 把一个 selected fact 插入平面树，并拒绝重复路径及叶/容器冲突。
fn insert_fact(root: &mut FactNode, segments: &[String], fact: ParsedFact) -> Result<(), ImError> {
    let mut node = root;
    for segment in segments {
        if node.scalar.is_some() {
            return Err(ImError::Parse(
                "category fact scalar cannot have children".to_string(),
            ));
        }
        node = node.children.entry(segment.to_owned()).or_default();
    }

    match fact {
        ParsedFact::Scalar(value) => {
            if node.scalar.is_some() || node.marker.is_some() || !node.children.is_empty() {
                return Err(ImError::Parse(
                    "category fact has duplicate or conflicting path".to_string(),
                ));
            }
            node.scalar = Some(value);
        }
        ParsedFact::Marker(kind) => {
            if node.scalar.is_some() || node.marker.is_some() {
                return Err(ImError::Parse(
                    "category fact has duplicate or conflicting marker".to_string(),
                ));
            }
            node.marker = Some(kind);
        }
    }
    Ok(())
}

impl FactNode {
    /// 按 marker 或 child pointer 形状递归构造 JSON，marker 永不覆盖孩子。
    fn build(self, depth: usize) -> Result<Value, ImError> {
        if depth > MAX_DEPTH {
            return Err(ImError::Parse(format!(
                "category fact reply nesting exceeds depth {MAX_DEPTH}"
            )));
        }
        if let Some(value) = self.scalar {
            if self.marker.is_some() || !self.children.is_empty() {
                return Err(ImError::Parse(
                    "category fact scalar cannot coexist with container".to_string(),
                ));
            }
            return Ok(value);
        }

        let kind = self
            .marker
            .or_else(|| infer_container_kind(&self.children))
            .ok_or_else(|| {
                ImError::Parse("category fact container type cannot be inferred".to_string())
            })?;
        match kind {
            ContainerKind::Object => {
                let mut object = Map::new();
                for (key, child) in self.children {
                    object.insert(key, child.build(depth + 1)?);
                }
                Ok(Value::Object(object))
            }
            ContainerKind::Array => {
                let mut children = Vec::with_capacity(self.children.len());
                for (key, child) in self.children {
                    let index = canonical_array_index(&key).ok_or_else(|| {
                        ImError::Parse(format!(
                            "category fact array child is not an index: {key:?}"
                        ))
                    })?;
                    children.push((index, child));
                }
                children.sort_unstable_by_key(|(index, _)| *index);
                let mut array = Vec::with_capacity(children.len());
                for (expected, (actual, child)) in children.into_iter().enumerate() {
                    if actual != expected {
                        return Err(ImError::Parse(
                            "category fact array indexes are not contiguous".to_string(),
                        ));
                    }
                    array.push(child.build(depth + 1)?);
                }
                Ok(Value::Array(array))
            }
        }
    }
}

/// 从 child key 形状推导容器类型；连续 canonical indexes 才能组成数组。
fn infer_container_kind(children: &BTreeMap<String, FactNode>) -> Option<ContainerKind> {
    if children.is_empty() {
        return None;
    }
    if children
        .keys()
        .all(|key| canonical_array_index(key).is_some())
    {
        let mut indexes = children
            .keys()
            .filter_map(|key| canonical_array_index(key))
            .collect::<Vec<_>>();
        indexes.sort_unstable();
        if indexes
            .iter()
            .enumerate()
            .all(|(expected, actual)| *actual == expected)
        {
            return Some(ContainerKind::Array);
        }
    }
    Some(ContainerKind::Object)
}

/// 解析数组索引 token；拒绝空 token、前导零和超出 bounded row 范围的索引。
fn canonical_array_index(segment: &str) -> Option<usize> {
    if segment == "0" {
        return Some(0);
    }
    if segment.is_empty()
        || segment.starts_with('0')
        || !segment.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let index = segment.parse::<usize>().ok()?;
    (index < MAX_FACT_ROWS).then_some(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 把 facts op 的 Text 行转换为 host rows codec 使用的 JSON bytes。
    fn reply(rows: &[Row]) -> Vec<u8> {
        let rows = rows
            .iter()
            .map(|row| {
                let object = row
                    .iter()
                    .map(|(column, value)| {
                        let value = match value {
                            SqlValue::Text(value) => Value::String(value.to_owned()),
                            SqlValue::Integer(value) => json!(value),
                            SqlValue::Real(value) => json!(value),
                            SqlValue::Null => Value::Null,
                            SqlValue::Blob(value) => json!(value),
                        };
                        (column.to_owned(), value)
                    })
                    .collect();
                Value::Object(object)
            })
            .collect::<Vec<_>>();
        serde_json::to_vec(&rows).expect("test rows JSON")
    }

    /// 提取 persist 生成的 fact rows，验证输入没有被作为整条 wire blob 保存。
    fn persisted_fact_rows(value: &Value) -> Vec<Row> {
        let ops = persist("scope", "7", value).expect("persist");
        let StorageOp::BatchUpsert(spec) = &ops[1] else {
            panic!("expected fact upsert")
        };
        spec.rows.clone()
    }

    #[test]
    /// Verify empty arrays and numeric object keys retain distinct JSON shapes.
    fn empty_array_marker_and_numeric_object_round_trip() {
        let value = json!({
            "items": [],
            "numericObject": {"0": "zero", "1": "one"},
            "values": [null, true, 3]
        });
        let rows = persisted_fact_rows(&value);
        assert!(rows.iter().any(|row| {
            row.iter().any(|(column, value)| {
                column == "path" && matches!(value, SqlValue::Text(path) if path == "/items")
            }) && row.iter().any(|(column, value)| {
                column == "value_json"
                    && matches!(value, SqlValue::Text(encoded) if encoded == "[]")
            })
        }));
        assert_eq!(decode(&reply(&rows), "7").expect("decode"), value);
    }

    #[test]
    /// Verify old scalar rows cannot resurrect removed fields.
    fn old_revision_tail_does_not_revive_removed_path() {
        let rows = vec![
            vec![
                (
                    "snapshot_key".to_string(),
                    SqlValue::Text("scope".to_string()),
                ),
                ("path".to_string(), SqlValue::Text("/keep".to_string())),
                ("revision".to_string(), SqlValue::Text("2".to_string())),
                ("value_json".to_string(), SqlValue::Text("2".to_string())),
            ],
            vec![
                (
                    "snapshot_key".to_string(),
                    SqlValue::Text("scope".to_string()),
                ),
                ("path".to_string(), SqlValue::Text("/gone".to_string())),
                ("revision".to_string(), SqlValue::Text("1".to_string())),
                (
                    "value_json".to_string(),
                    SqlValue::Text("\"old\"".to_string()),
                ),
            ],
        ];
        assert_eq!(
            decode(&reply(&rows), "2").expect("decode"),
            json!({"keep": 2})
        );
    }

    #[test]
    /// Verify malformed durable facts are rejected rather than partially decoded.
    fn malformed_rows_fail_closed() {
        assert!(decode(br#"{}"#, "1").is_err());
        assert!(decode(
            br#"[{"snapshot_key":"scope","path":"/bad~2path","revision":"1","value_json":"1"}]"#,
            "1"
        )
        .is_err());
        assert!(decode(
            br#"[{"snapshot_key":"scope","path":"/x","revision":"1","value_json":"{"}]"#,
            "1"
        )
        .is_err());
    }

    #[test]
    /// Verify decimal revisions preserve the full u64 range.
    fn revisions_are_exact_decimal_without_float_rounding() {
        assert_eq!(
            revision_cmp("9007199254740993", "9007199254740992").expect("compare"),
            Ordering::Greater
        );
        assert!(revision_cmp("01", "1").is_err());
        assert!(revision_cmp("18446744073709551616", "1").is_err());
    }

    #[test]
    /// Verify persistence stores normalized scalar facts instead of one wire blob.
    fn persistence_keeps_scalar_rows_instead_of_wire_blob() {
        let value = json!({"title":"title","nested":{"answer":42}});
        let rows = persisted_fact_rows(&value);
        let whole = serde_json::to_string(&value).expect("wire JSON");
        assert!(rows.iter().all(|row| {
            row.iter().all(|(column, value)| {
                column != "value_json"
                    || !matches!(value, SqlValue::Text(encoded) if encoded == &whole)
            })
        }));
    }
}
