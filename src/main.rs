use chrono::DateTime;
use clap::{App, Arg};
use failure::Error;
use jsl::{Form, Schema, Type};
use serde_json::Value;

use std::collections::HashMap;
use std::fs::File;
use std::io::stdin;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Read;

fn main() -> Result<(), Error> {
    let matches = App::new("jsl-infer")
        .version("0.1")
        .about("Infers a JSON Schema Language schema from example JSON values")
        .arg(
            Arg::with_name("INPUT")
                .help("Where to read examples from. Dash (hypen) indicates stdin")
                .default_value("-"),
        )
        .get_matches();

    let reader = BufReader::new(match matches.value_of("INPUT").unwrap() {
        "-" => Box::new(stdin()) as Box<Read>,
        file @ _ => Box::new(File::open(file)?) as Box<Read>,
    });

    let mut inference = InferredSchema::Unknown;
    for line in reader.lines() {
        inference = inference.infer(serde_json::from_str(&line?)?);
    }

    let serde_schema = inference.into_schema().into_serde();
    println!("{}", serde_json::to_string(&serde_schema)?);

    Ok(())
}

#[derive(Debug)]
enum InferredSchema {
    Unknown,
    Any,
    Bool,
    Number,
    Timestamp,
    String,
    Array(Box<InferredSchema>),
    Properties(Box<InferredProperties>),
}

#[derive(Debug)]
struct InferredProperties {
    required: HashMap<String, InferredSchema>,
    optional: HashMap<String, InferredSchema>,
}

impl InferredSchema {
    fn infer(self, value: Value) -> InferredSchema {
        match (self, value) {
            (InferredSchema::Unknown, Value::Null) => InferredSchema::Any,
            (InferredSchema::Unknown, Value::Bool(_)) => InferredSchema::Bool,
            (InferredSchema::Unknown, Value::Number(_)) => InferredSchema::Number,
            (InferredSchema::Unknown, Value::String(s)) => {
                if DateTime::parse_from_rfc3339(&s).is_ok() {
                    InferredSchema::Timestamp
                } else {
                    InferredSchema::String
                }
            }
            (InferredSchema::Unknown, Value::Array(vals)) => {
                let mut sub_infer = InferredSchema::Unknown;
                for v in vals {
                    sub_infer = sub_infer.infer(v);
                }

                InferredSchema::Array(Box::new(sub_infer))
            }
            (InferredSchema::Unknown, Value::Object(map)) => {
                let mut props = HashMap::new();
                for (k, v) in map {
                    props.insert(k, InferredSchema::Unknown.infer(v));
                }

                InferredSchema::Properties(Box::new(InferredProperties {
                    required: props,
                    optional: HashMap::new(),
                }))
            }
            (InferredSchema::Any, _) => InferredSchema::Any,
            (InferredSchema::Bool, Value::Bool(_)) => InferredSchema::Bool,
            (InferredSchema::Bool, _) => InferredSchema::Any,
            (InferredSchema::Number, Value::Number(_)) => InferredSchema::Number,
            (InferredSchema::Number, _) => InferredSchema::Any,
            (InferredSchema::Timestamp, Value::String(s)) => {
                if DateTime::parse_from_rfc3339(&s).is_ok() {
                    InferredSchema::Timestamp
                } else {
                    InferredSchema::String
                }
            }
            (InferredSchema::Timestamp, _) => InferredSchema::Any,
            (InferredSchema::String, Value::String(_)) => InferredSchema::String,
            (InferredSchema::String, _) => InferredSchema::Any,
            (InferredSchema::Array(prior), Value::Array(vals)) => {
                let mut sub_infer = *prior;
                for v in vals {
                    sub_infer = sub_infer.infer(v);
                }

                InferredSchema::Array(Box::new(sub_infer))
            }
            (InferredSchema::Array(_), _) => InferredSchema::Any,
            (InferredSchema::Properties(mut prior), Value::Object(map)) => {
                let missing_required_keys: Vec<_> = prior
                    .required
                    .keys()
                    .filter(|k| !map.contains_key(k.clone()))
                    .cloned()
                    .collect();
                for k in missing_required_keys {
                    let sub_prior = prior.required.remove(&k).unwrap();
                    prior.optional.insert(k, sub_prior);
                }

                for (k, v) in map {
                    if prior.required.contains_key(&k) {
                        let sub_prior = prior.required.remove(&k).unwrap().infer(v);
                        prior.required.insert(k, sub_prior);
                    } else if prior.optional.contains_key(&k) {
                        let sub_prior = prior.optional.remove(&k).unwrap().infer(v);
                        prior.optional.insert(k, sub_prior);
                    } else {
                        prior.optional.insert(k, InferredSchema::Unknown.infer(v));
                    }
                }

                InferredSchema::Properties(prior)
            }
            (InferredSchema::Properties(_), _) => InferredSchema::Any,
        }
    }

    fn into_schema(self) -> Schema {
        let form = match self {
            InferredSchema::Unknown => Form::Empty,
            InferredSchema::Any => Form::Empty,
            InferredSchema::Bool => Form::Type(Type::Boolean),
            InferredSchema::Number => Form::Type(Type::Number),
            InferredSchema::String => Form::Type(Type::String),
            InferredSchema::Timestamp => Form::Type(Type::Timestamp),
            InferredSchema::Array(sub_prior) => Form::Elements(sub_prior.into_schema()),
            InferredSchema::Properties(props) => {
                let has_required = !props.required.is_empty();

                Form::Properties(
                    props
                        .required
                        .into_iter()
                        .map(|(k, v)| (k, v.into_schema()))
                        .collect(),
                    props
                        .optional
                        .into_iter()
                        .map(|(k, v)| (k, v.into_schema()))
                        .collect(),
                    has_required,
                )
            }
        };

        Schema::from_parts(None, Box::new(form), HashMap::new())
    }
}
