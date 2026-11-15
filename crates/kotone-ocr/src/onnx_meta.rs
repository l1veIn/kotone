//! 读取 ONNX ModelProto 顶层 `metadata_props`（字段 14，StringStringEntryProto{key=1,value=2}）。
//!
//! 只做最小的 protobuf 线格式遍历，用于取出识别模型内嵌的字符表（key=`character`）。
//! 不解析计算图；任何越界或非法编码都返回错误而不是 panic。

fn read_varint(bytes: &[u8], pos: &mut usize) -> Result<u64, String> {
    let mut value: u64 = 0;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(*pos).ok_or("varint 越界")?;
        *pos += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("varint 过长".into())
}

fn read_len<'a>(bytes: &'a [u8], pos: &mut usize) -> Result<&'a [u8], String> {
    let len = read_varint(bytes, pos)? as usize;
    let end = pos.checked_add(len).ok_or("长度溢出")?;
    let slice = bytes.get(*pos..end).ok_or("长度越界")?;
    *pos = end;
    Ok(slice)
}

fn skip(bytes: &[u8], pos: &mut usize, wire: u64) -> Result<(), String> {
    match wire {
        0 => {
            read_varint(bytes, pos)?;
        }
        1 => {
            *pos = pos
                .checked_add(8)
                .filter(|p| *p <= bytes.len())
                .ok_or("fixed64 越界")?
        }
        2 => {
            read_len(bytes, pos)?;
        }
        5 => {
            *pos = pos
                .checked_add(4)
                .filter(|p| *p <= bytes.len())
                .ok_or("fixed32 越界")?
        }
        other => return Err(format!("不支持的 wire type {other}")),
    }
    Ok(())
}

/// 返回全部 metadata_props 键值对。
pub fn metadata_props(model: &[u8]) -> Result<Vec<(String, String)>, String> {
    let mut pos = 0;
    let mut out = Vec::new();
    while pos < model.len() {
        let tag = read_varint(model, &mut pos)?;
        let (field, wire) = (tag >> 3, tag & 7);
        if field == 14 && wire == 2 {
            let entry = read_len(model, &mut pos)?;
            let mut p = 0;
            let (mut key, mut value) = (String::new(), String::new());
            while p < entry.len() {
                let tag = read_varint(entry, &mut p)?;
                match (tag >> 3, tag & 7) {
                    (1, 2) => key = String::from_utf8_lossy(read_len(entry, &mut p)?).into_owned(),
                    (2, 2) => {
                        value = String::from_utf8(read_len(entry, &mut p)?.to_vec())
                            .map_err(|_| "metadata 值不是 UTF-8")?
                    }
                    (_, w) => skip(entry, &mut p, w)?,
                }
            }
            out.push((key, value));
        } else {
            skip(model, &mut pos, wire)?;
        }
    }
    Ok(out)
}

/// 取识别模型内嵌字符表（按行），不含 blank 与末尾空格（由解码器追加）。
pub fn character_list(model: &[u8]) -> Result<Vec<String>, String> {
    let props = metadata_props(model)?;
    let raw = props
        .into_iter()
        .find(|(k, _)| k == "character")
        .map(|(_, v)| v)
        .ok_or("识别模型缺少内嵌字符表（metadata `character`）")?;
    let list: Vec<String> = raw
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
        .collect();
    // 末尾换行产生的空项去掉；中间的空行（若有）原样保留以维持索引
    let mut list = list;
    while list.last().is_some_and(|s| s.is_empty()) {
        list.pop();
    }
    if list.is_empty() {
        return Err("内嵌字符表为空".into());
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varint(mut v: u64, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                break;
            }
            out.push(b | 0x80);
        }
    }

    fn field_bytes(field: u64, data: &[u8], out: &mut Vec<u8>) {
        varint(field << 3 | 2, out);
        varint(data.len() as u64, out);
        out.extend_from_slice(data);
    }

    #[test]
    fn parses_metadata_and_skips_other_fields() {
        let mut model = Vec::new();
        varint(1 << 3, &mut model); // ir_version varint
        varint(8, &mut model);
        field_bytes(7, &[0xde, 0xad, 0xbe, 0xef], &mut model); // graph（跳过）
        varint(9 << 3 | 5, &mut model); // fixed32
        model.extend_from_slice(&[1, 2, 3, 4]);
        let mut entry = Vec::new();
        field_bytes(1, b"character", &mut entry);
        field_bytes(2, "가\n나\r\n다\n".as_bytes(), &mut entry);
        field_bytes(14, &entry, &mut model);
        let mut other = Vec::new();
        field_bytes(1, b"shape", &mut other);
        field_bytes(2, b"[3,48,320]", &mut other);
        field_bytes(14, &other, &mut model);
        let props = metadata_props(&model).unwrap();
        assert_eq!(props.len(), 2);
        assert_eq!(character_list(&model).unwrap(), vec!["가", "나", "다"]);
    }

    #[test]
    fn malformed_input_is_an_error_not_a_panic() {
        assert!(metadata_props(&[0x72, 0xff]).is_err()); // 长度越界
        assert!(metadata_props(&[0x0f]).is_err()); // wire type 7
        assert!(metadata_props(&[0xff; 11]).is_err()); // varint 过长
        assert!(character_list(&[]).is_err());
    }
}
