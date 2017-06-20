// Copyright 2017 PingCAP, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// See the License for the specific language governing permissions and
// limitations under the License.

// FIXME: remove following later
#![allow(dead_code)]

use std::{u32, char};
use std::collections::BTreeMap;
use super::super::Result;
use super::json::Json;
use super::path_expr::{PathLeg, PathExpression, PATH_EXPR_ASTERISK, PATH_EXPR_ARRAY_INDEX_ASTERISK};

const ESCAPED_UNICODE_BYTES_SIZE: usize = 4;

impl Json {
    // extract receives several path expressions as arguments, matches them in j, and returns
    // the target JSON matched any path expressions, which may be autowrapped as an array.
    // If there is no any expression matched, it returns None.
    pub fn extract(&self, path_expr_list: &[PathExpression]) -> Option<Json> {
        let mut elem_list = Vec::with_capacity(path_expr_list.len());
        for path_expr in path_expr_list {
            elem_list.append(&mut extract_json(self.clone(), path_expr))
        }
        if elem_list.is_empty() {
            return None;
        }
        if path_expr_list.len() == 1 && elem_list.len() == 1 {
            // If path_expr contains asterisks, elem_list.len() won't be 1
            // even if path_expr_list.len() equals to 1.
            return Some(elem_list.remove(0));
        }
        Some(Json::Array(elem_list))
    }

    pub fn unquote(&self) -> Result<String> {
        match *self {
            Json::String(ref s) => unquote_string(s),
            _ => Ok(format!("{:?}", self)),
        }
    }
}

// unquote_string recognizes the escape sequences shown in:
// https://dev.mysql.com/doc/refman/5.7/en/json-modification-functions.html#
// json-unquote-character-escape-sequences
pub fn unquote_string(s: &str) -> Result<String> {
    let mut ret = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let c = match chars.next() {
                Some(c) => c,
                None => return Err(box_err!("Missing a closing quotation mark in string")),
            };
            match c {
                '"' => ret.push('"'),
                'b' => ret.push('\x08'),
                'f' => ret.push('\x0C'),
                'n' => ret.push('\x0A'),
                'r' => ret.push('\x0D'),
                't' => ret.push('\x0B'),
                '\\' => ret.push('\\'),
                'u' => {
                    let mut unicode = String::with_capacity(ESCAPED_UNICODE_BYTES_SIZE);
                    for _ in 0..ESCAPED_UNICODE_BYTES_SIZE {
                        match chars.next() {
                            Some(c) => unicode.push(c),
                            None => return Err(box_err!("Invalid unicode: {}", unicode)),
                        }
                    }
                    let utf8 = try!(decode_escaped_unicode(&unicode));
                    ret.push(utf8);
                }
                _ => ret.push(c),
            }
        } else {
            ret.push(ch);
        }
    }
    Ok(ret)
}

fn decode_escaped_unicode(s: &str) -> Result<char> {
    let u = box_try!(u32::from_str_radix(s, 16));
    char::from_u32(u).ok_or(box_err!("invalid char from: {}", s))
}

// extract_json is used by JSON::extract().
pub fn extract_json(j: Json, path_expr: &PathExpression) -> Vec<Json> {
    if path_expr.legs.is_empty() {
        return vec![j.clone()];
    }
    let (current_leg, sub_path_expr) = path_expr.pop_one_leg();
    let mut ret = vec![];
    match current_leg {
        PathLeg::Index(i) => {
            // If j is not an array, autowrap that into array.
            let array = match j {
                Json::Array(array) => array,
                _ => wrap_to_array(j),
            };
            if i == PATH_EXPR_ARRAY_INDEX_ASTERISK {
                for child in array {
                    ret.append(&mut extract_json(child, &sub_path_expr))
                }
            } else if (i as usize) < array.len() {
                ret.append(&mut extract_json(array[i as usize].clone(), &sub_path_expr))
            }
        }
        PathLeg::Key(key) => {
            if let Json::Object(map) = j {
                if key == PATH_EXPR_ASTERISK {
                    let sorted_keys = get_sorted_keys(&map);
                    for key in sorted_keys {
                        ret.append(&mut extract_json(map[&key].clone(), &sub_path_expr))
                    }
                } else if map.contains_key(&key) {
                    ret.append(&mut extract_json(map[&key].clone(), &sub_path_expr))
                }
            }
        }
        PathLeg::DoubleAsterisk => {
            ret.append(&mut extract_json(j.clone(), &sub_path_expr));
            match j {
                Json::Array(array) => {
                    for child in array {
                        ret.append(&mut extract_json(child.clone(), path_expr))
                    }
                }
                Json::Object(map) => {
                    let sorted_keys = get_sorted_keys(&map);
                    for key in sorted_keys {
                        ret.append(&mut extract_json(map[&key].clone(), path_expr))
                    }
                }
                _ => {}
            }
        }
    }
    ret
}

fn wrap_to_array(j: Json) -> Vec<Json> {
    let mut array = Vec::with_capacity(1);
    array.push(j.clone());
    array
}

// Get_sorted_keys returns sorted keys of a map.
fn get_sorted_keys(m: &BTreeMap<String, Json>) -> Vec<String> {
    let mut keys = Vec::with_capacity(m.len());
    for k in m.keys() {
        keys.push(k.clone());
    }
    keys.sort();
    keys
}

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;
    use super::*;
    use super::super::path_expr::{PathExpressionFlag, PATH_EXPR_ARRAY_INDEX_ASTERISK,
                                  PATH_EXPRESSION_CONTAINS_ASTERISK,
                                  PATH_EXPRESSION_CONTAINS_DOUBLE_ASTERISK};

    #[test]
    fn test_get_sorted_keys() {
        let mut m = BTreeMap::new();
        let keys = ["a", "b", "c"];
        for k in &keys {
            m.insert(String::from(*k), Json::None);
        }
        let expected: Vec<_> = keys.iter().map(|x| String::from(*x)).collect();
        assert_eq!(super::get_sorted_keys(&m), expected);
    }

    #[test]
    fn test_json_extract() {
        let mut m = BTreeMap::new();
        m.insert(String::from("a"), Json::String(String::from("a1")));
        m.insert(String::from("b"), Json::Double(20.08));
        m.insert(String::from("c"), Json::Boolean(false));
        let mut mm = BTreeMap::new();
        mm.insert(String::from("g"), Json::Object(m.clone()));
        let mut test_cases =
            vec![// no path expression
                 (Json::None, vec![], None),
                 // Index
                 (Json::Array(vec![Json::Boolean(true), Json::I64(2017)]),
                  vec![PathExpression {
                           legs: vec![PathLeg::Index(0)],
                           flags: PathExpressionFlag::default(),
                       }],
                  Some(Json::Boolean(true))),
                 (Json::Array(vec![Json::Boolean(true), Json::I64(2017)]),
                  vec![PathExpression {
                           legs: vec![PathLeg::Index(PATH_EXPR_ARRAY_INDEX_ASTERISK)],
                           flags: PATH_EXPRESSION_CONTAINS_ASTERISK,
                       }],
                  Some(Json::Array(vec![Json::Boolean(true), Json::I64(2017)]))),
                 (Json::Array(vec![Json::Boolean(true), Json::I64(2017)]),
                  vec![PathExpression {
                           legs: vec![PathLeg::Index(2)],
                           flags: PathExpressionFlag::default(),
                       }],
                  None),
                 (Json::Double(6.18),
                  vec![PathExpression {
                           legs: vec![PathLeg::Index(0)],
                           flags: PathExpressionFlag::default(),
                       }],
                  Some(Json::Double(6.18))),
                 // Key
                 (Json::Object(m.clone()),
                  vec![PathExpression {
                           legs: vec![PathLeg::Key(String::from("c"))],
                           flags: PathExpressionFlag::default(),
                       }],
                  Some(Json::Boolean(false))),
                 (Json::Object(m.clone()),
                  vec![PathExpression {
                           legs: vec![PathLeg::Key(String::from(PATH_EXPR_ASTERISK))],
                           flags: PATH_EXPRESSION_CONTAINS_ASTERISK,
                       }],
                  Some(Json::Array(vec![Json::String(String::from("a1")),
                                        Json::Double(20.08),
                                        Json::Boolean(false)]))),
                 (Json::Object(m.clone()),
                  vec![PathExpression {
                           legs: vec![PathLeg::Key(String::from("d"))],
                           flags: PathExpressionFlag::default(),
                       }],
                  None),
                 // Double asterisks
                 (Json::I64(21),
                  vec![PathExpression {
                           legs: vec![PathLeg::DoubleAsterisk, PathLeg::Key(String::from("c"))],
                           flags: PATH_EXPRESSION_CONTAINS_DOUBLE_ASTERISK,
                       }],
                  None),
                 (Json::Object(mm),
                  vec![PathExpression {
                           legs: vec![PathLeg::DoubleAsterisk, PathLeg::Key(String::from("c"))],
                           flags: PATH_EXPRESSION_CONTAINS_DOUBLE_ASTERISK,
                       }],
                  Some(Json::Boolean(false))),
                 (Json::Array(vec![Json::Object(m), Json::Boolean(true)]),
                  vec![PathExpression {
                           legs: vec![PathLeg::DoubleAsterisk, PathLeg::Key(String::from("c"))],
                           flags: PATH_EXPRESSION_CONTAINS_DOUBLE_ASTERISK,
                       }],
                  Some(Json::Boolean(false)))];
        for (i, (j, exprs, expected)) in test_cases.drain(..).enumerate() {
            let got = j.extract(&exprs[..]);
            assert_eq!(got,
                       expected,
                       "#{} expect {:?}, but got {:?}",
                       i,
                       expected,
                       got);
        }
    }

    #[test]
    fn test_decode_escaped_unicode() {
        let mut test_cases = vec![
            ("5e8a", '床'),
            ("524d", '前'),
            ("660e", '明'),
            ("6708", '月'),
            ("5149", '光'),
        ];
        for (i, (escaped, expected)) in test_cases.drain(..).enumerate() {
            let d = decode_escaped_unicode(escaped);
            assert!(d.is_ok(), "#{} expect ok but got err {:?}", i, d);
            let got = d.unwrap();
            assert_eq!(got,
                       expected,
                       "#{} expect {:?} but got {:?}",
                       i,
                       expected,
                       got);
        }
    }

    #[test]
    fn test_json_unquote() {
        // test unquote json string
        let mut test_cases = vec![("\\b", true, Some("\x08")),
                                  ("\\f", true, Some("\x0C")),
                                  ("\\n", true, Some("\x0A")),
                                  ("\\r", true, Some("\x0D")),
                                  ("\\t", true, Some("\x0B")),
                                  ("\\\\", true, Some("\x5c")),
                                  ("\\u597d", true, Some("好")),
                                  ("0\\u597d0", true, Some("0好0")),
                                  ("[", true, Some("[")),
                                  // invalid input
                                  ("\\", false, None),
                                  ("\\u59", false, None)];
        for (i, (input, no_error, expected)) in test_cases.drain(..).enumerate() {
            let j = Json::String(String::from(input));
            let r = j.unquote();
            if no_error {
                assert!(r.is_ok(), "#{} expect unquote ok but got err {:?}", i, r);
                let got = r.unwrap();
                let expected = String::from(expected.unwrap());
                assert_eq!(got,
                           expected,
                           "#{} expect {:?} but got {:?}",
                           i,
                           expected,
                           got);
            } else {
                assert!(r.is_err(), "#{} expected error but got {:?}", i, r);
            }
        }

        // test unquote other json types
        let mut test_cases = vec![Json::Object(BTreeMap::new()),
                                  Json::Array(vec![]),
                                  Json::I64(2017),
                                  Json::Double(19.28),
                                  Json::Boolean(true),
                                  Json::None];
        for (i, j) in test_cases.drain(..).enumerate() {
            let expected = format!("{:?}", j);
            let r = j.unquote();
            assert!(r.is_ok(), "#{} expect unquote ok but got err {:?}", i, r);
            let got = r.unwrap();
            assert_eq!(got,
                       expected,
                       "#{} expect {:?} but got {:?}",
                       i,
                       expected,
                       got);
        }
    }
}
