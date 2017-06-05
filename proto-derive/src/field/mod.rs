mod map;
mod message;
mod oneof;
mod scalar;

use std::fmt;
use std::slice;

use quote::Tokens;
use syn::{
    Attribute,
    Ident,
    Lit,
    MetaItem,
    NestedMetaItem,
};

use error::*;

pub enum Field {
    /// A scalar field.
    Scalar(scalar::Field),
    /// A message field.
    Message(message::Field),
    /// A map field.
    Map(map::Field),
    /// A oneof field.
    Oneof(oneof::Field),
}

impl Field {

    /// Creates a new `Field` from an iterator of field attributes.
    ///
    /// If the meta items are invalid, an error will be returned.
    /// If the field should be ignored, `None` is returned.
    pub fn new(attrs: Vec<Attribute>) -> Result<Option<Field>> {
        // Get the items belonging to the 'proto' list attribute (e.g. #[proto(foo, bar="baz")]).
        let attrs: Vec<MetaItem> = attrs.into_iter().flat_map(|attr| match attr.value {
            MetaItem::List(ident, items) => if ident == "proto" { items } else { Vec::new() },
            _ => Vec::new(),
        }).flat_map(|attr| -> Result<_> {
            match attr {
                NestedMetaItem::MetaItem(attr) => Ok(attr),
                NestedMetaItem::Literal(lit) => bail!("invalid proto attribute: {:?}", lit),
            }
        }).collect();

        // TODO: check for ignore attribute.

        let field = if let Some(field) = scalar::Field::new(&attrs)? {
            Field::Scalar(field)
        } else if let Some(field) = message::Field::new(&attrs)? {
            Field::Message(field)
        } else if let Some(field) = map::Field::new(&attrs)? {
            Field::Map(field)
        } else if let Some(field) = oneof::Field::new(&attrs)? {
            Field::Oneof(field)
        } else {
            bail!("no type attribute");
        };

        Ok(Some(field))
    }

    pub fn tags(&self) -> Vec<u32> {
        match *self {
            Field::Scalar(ref scalar) => vec![scalar.tag],
            Field::Message(ref message) => vec![message.tag],
            Field::Map(ref map) => vec![map.tag],
            Field::Oneof(ref oneof) => oneof.tags.clone(),
        }
    }

    /// Returns an expression which evaluates to the encoded length of the field.
    pub fn encode(&self, ident: &Ident) -> Tokens {
        match *self {
            Field::Scalar(ref scalar) => scalar.encode(ident),
            Field::Message(ref message) => message.encode(ident),
            Field::Map(ref map) => map.encode(ident),
            Field::Oneof { .. } => quote!(();),
        }
    }

    /// Returns an expression which evaluates to the result of merging a decoded
    /// value into the field.
    pub fn merge(&self, ident: &Ident) -> Tokens {
        match *self {
            Field::Scalar(ref scalar) => scalar.merge(ident),
            Field::Message(ref message) => message.merge(ident),
            Field::Map(ref map) => map.merge(ident),
            _ => quote!(Ok(())),
        }
    }

    /// Returns an expression which evaluates to the encoded length of the field.
    pub fn encoded_len(&self, ident: &Ident) -> Tokens {
        match *self {
            Field::Scalar(ref scalar) => scalar.encoded_len(ident),
            Field::Map(ref map) => map.encoded_len(ident),
            Field::Message(ref msg) => msg.encoded_len(ident),
            _ => quote!(0),
        }
    }

    pub fn default(&self) -> Tokens {
        match *self {
            Field::Scalar(ref scalar) => scalar.default(),
            _ => quote!(::std::default::Default::default()),
        }
    }

    pub fn methods(&self, ident: &Ident) -> Option<Tokens> {
        match *self {
            Field::Scalar(ref scalar) => scalar.methods(ident),
            Field::Map(ref map) => map.methods(ident),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Label {
    /// An optional field.
    Optional,
    /// A required field.
    Required,
    /// A repeated field.
    Repeated,
}

impl Label {
    fn as_str(&self) -> &'static str {
        match *self {
            Label::Optional => "optional",
            Label::Required => "required",
            Label::Repeated => "repeated",
        }
    }

    fn variants() -> slice::Iter<'static, Label> {
        const VARIANTS: &'static [Label] = &[
            Label::Optional,
            Label::Required,
            Label::Repeated,
        ];
        VARIANTS.iter()
    }

    /// Parses a string into a field label.
    /// If the string doesn't match a field label, `None` is returned.
    fn from_attr(attr: &MetaItem) -> Option<Label> {
        if let MetaItem::Word(ref ident) = *attr {
            for &label in Label::variants() {
                if ident == label.as_str() {
                    return Some(label);
                }
            }
        }
        None
    }
}

impl fmt::Debug for Label {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub fn set_option<T>(option: &mut Option<T>, value: T, message: &str) -> Result<()>
where T: fmt::Debug {
    if let Some(ref existing) = *option {
        bail!("{}: {:?} and {:?}", message, existing, value);
    }
    *option = Some(value);
    Ok(())
}

pub fn set_bool(b: &mut bool, message: &str) -> Result<()> {
    if *b {
        bail!(message);
    } else {
        *b = true;
        Ok(())
    }
}


/// Unpacks an attribute into a (key, boolean) pair, returning the boolean value.
/// If the key doesn't match the attribute, `None` is returned.
fn bool_attr(key: &str, attr: &MetaItem) -> Result<Option<bool>> {
    if attr.name() != key {
        return Ok(None);
    }
    match *attr {
        MetaItem::Word(..) => Ok(Some(true)),
        MetaItem::List(_, ref items) => {
            // TODO(rustlang/rust#23121): slice pattern matching would make this much nicer.
            if items.len() == 1 {
                if let NestedMetaItem::Literal(Lit::Bool(value)) = items[0] {
                    return Ok(Some(value))
                }
            }
            bail!("invalid {} attribute", key);
        },
        MetaItem::NameValue(_, Lit::Str(ref s, _)) => {
            s.parse::<bool>().map_err(|e| Error::from(e.to_string())).map(Option::Some)
        },
        MetaItem::NameValue(_, Lit::Bool(value)) => Ok(Some(value)),
        _ => bail!("invalid {} attribute", key),
    }
}

/// Checks if an attribute matches a word.
fn word_attr(key: &str, attr: &MetaItem) -> bool {
    if let MetaItem::Word(ref ident) = *attr {
        ident == key
    } else {
        false
    }
}

fn tag_attr(attr: &MetaItem) -> Result<Option<u32>> {
    if attr.name() != "tag" {
        return Ok(None);
    }
    match *attr {
        MetaItem::List(_, ref items) => {
            // TODO(rustlang/rust#23121): slice pattern matching would make this much nicer.
            if items.len() == 1 {
                if let NestedMetaItem::Literal(Lit::Int(value, _)) = items[0] {
                    return Ok(Some(value as u32));
                }
            }
            bail!("invalid tag attribute: {:?}", attr);
        },
        MetaItem::NameValue(_, ref lit) => {
            match *lit {
                Lit::Str(ref s, _) => s.parse::<u32>().map_err(|e| Error::from(e.to_string()))
                                                      .map(Option::Some),
                Lit::Int(value, _) => return Ok(Some(value as u32)),
                _ => bail!("invalid tag attribute: {:?}", attr),
            }
        },
        _ => bail!("invalid tag attribute: {:?}", attr),
    }
}

fn tags_attr(attr: &MetaItem) -> Result<Option<Vec<u32>>> {
    if attr.name() != "tags" {
        return Ok(None);
    }
    match *attr {
        MetaItem::List(_, ref items) => {
            let mut tags = Vec::with_capacity(items.len());
            for item in items {
                if let Some(&NestedMetaItem::Literal(Lit::Int(value, _))) = items.first() {
                    tags.push(value as u32);
                } else {
                    bail!("invalid tag attribute: {:?}", attr);
                }
            }
            return Ok(Some(tags));
        },
        MetaItem::NameValue(_, Lit::Str(ref s, _)) => {
            s.split(',')
             .map(|s| s.trim().parse::<u32>().map_err(|e| Error::from(e.to_string())))
             .collect::<Result<Vec<u32>>>()
             .map(|tags| Some(tags))
        },
        _ => bail!("invalid tag attribute: {:?}", attr),
    }
}
