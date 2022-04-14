// Copyright 2019 CoreOS, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Serde serializer for a Parser struct, producing a Vec of command-line
//! arguments.

use anyhow::Context;
use clap::Parser;
use serde::{ser, Serialize};

pub(super) fn to_args<T>(value: &T) -> anyhow::Result<Vec<String>>
where
    T: Serialize + Parser,
{
    // We need to be able to find out whether a field is an --option
    // or a positional argument.  clap doesn't provide an API for this,
    // and we don't want to implement a proc macro because those have to
    // go in a separate crate.  Get the subcommand help text and grep it.
    let mut help = Vec::new();
    T::command()
        .write_long_help(&mut help)
        .context("reading subcommand help text")?;

    let mut serializer = Serializer {
        help_text: String::from_utf8(help)
            .context("decoding subcommand help text")?
            // add trailing space to each line, so push_field()
            // option check works consistently for boolean flags
            .replace('\n', " \n"),
        output: Vec::new(),
        field_stack: Vec::new(),
    };
    value.serialize(&mut serializer)?;
    Ok(serializer.output)
}

struct Serializer {
    help_text: String,
    output: Vec<String>,
    field_stack: Vec<Option<&'static str>>,
}

impl Serializer {
    fn push_field(&mut self, name: &'static str) {
        let field = if self.help_text.contains(&format!(" --{} ", name)) {
            Some(name)
        } else {
            // don't serialize to --option
            None
        };
        self.field_stack.push(field);
    }

    fn pop_field(&mut self) {
        self.field_stack.pop();
    }

    fn output_option(&mut self) {
        match &self.field_stack[self.field_stack.len() - 1] {
            None => (),
            Some(name) => {
                let option = format!("--{}", name);
                self.output_argument(option);
            }
        }
    }

    fn output_argument<T: ToString>(&mut self, arg: T) {
        self.output.push(arg.to_string());
    }
}

// Enormous pile of boilerplate covering every basic type.  We only need
// to handle a few:
// - The containing struct => walk each field, tracking option names
// - Sequences => add an option argument before each Vec entry
// - Options => serialize the wrapped value if Some
// - Bools => add option argument only if true
// - String/number primitives => add option argument, then value
// https://serde.rs/impl-serializer.html
// https://docs.serde.rs/serde/trait.Serializer.html
impl<'a> ser::Serializer for &'a mut Serializer {
    type Ok = ();
    type Error = SerializeError;
    type SerializeSeq = Self;
    type SerializeStruct = Self;
    type SerializeMap = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeStructVariant = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeTuple = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeTupleStruct = ser::Impossible<Self::Ok, Self::Error>;
    type SerializeTupleVariant = ser::Impossible<Self::Ok, Self::Error>;

    fn serialize_bool(self, v: bool) -> Result<()> {
        if v {
            self.output_option();
        }
        Ok(())
    }

    fn serialize_i8(self, v: i8) -> Result<()> {
        self.serialize_i64(i64::from(v))
    }

    fn serialize_i16(self, v: i16) -> Result<()> {
        self.serialize_i64(i64::from(v))
    }

    fn serialize_i32(self, v: i32) -> Result<()> {
        self.serialize_i64(i64::from(v))
    }

    fn serialize_i64(self, v: i64) -> Result<()> {
        self.output_option();
        self.output_argument(v);
        Ok(())
    }

    fn serialize_u8(self, v: u8) -> Result<()> {
        self.serialize_u64(u64::from(v))
    }

    fn serialize_u16(self, v: u16) -> Result<()> {
        self.serialize_u64(u64::from(v))
    }

    fn serialize_u32(self, v: u32) -> Result<()> {
        self.serialize_u64(u64::from(v))
    }

    fn serialize_u64(self, v: u64) -> Result<()> {
        self.output_option();
        self.output_argument(v);
        Ok(())
    }

    fn serialize_f32(self, v: f32) -> Result<()> {
        self.serialize_f64(f64::from(v))
    }

    fn serialize_f64(self, v: f64) -> Result<()> {
        self.output_option();
        self.output_argument(v);
        Ok(())
    }

    fn serialize_char(self, v: char) -> Result<()> {
        self.serialize_str(&v.to_string())
    }

    fn serialize_str(self, v: &str) -> Result<()> {
        self.output_option();
        self.output_argument(v);
        Ok(())
    }

    fn serialize_bytes(self, _v: &[u8]) -> Result<()> {
        unimplemented!()
    }

    fn serialize_none(self) -> Result<()> {
        Ok(())
    }

    fn serialize_some<T>(self, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(self)
    }

    // Anonymous value containing no data
    fn serialize_unit(self) -> Result<()> {
        Ok(())
    }

    // Named value containing no data
    fn serialize_unit_struct(self, _name: &'static str) -> Result<()> {
        Ok(())
    }

    // Unit enum variant
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
    ) -> Result<()> {
        unimplemented!()
    }

    fn serialize_newtype_struct<T>(self, _name: &'static str, _value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        unimplemented!()
    }

    fn serialize_newtype_variant<T>(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _value: &T,
    ) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        unimplemented!()
    }

    fn serialize_seq(self, _len: Option<usize>) -> Result<Self::SerializeSeq> {
        // no setup to be done
        Ok(self)
    }

    fn serialize_tuple(self, _len: usize) -> Result<Self::SerializeTuple> {
        unimplemented!();
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleStruct> {
        unimplemented!();
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant> {
        unimplemented!();
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<Self::SerializeMap> {
        unimplemented!();
    }

    fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<Self::SerializeStruct> {
        // no setup to be done
        Ok(self)
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant> {
        unimplemented!();
    }
}

impl<'a> ser::SerializeSeq for &'a mut Serializer {
    type Ok = ();
    type Error = SerializeError;

    fn serialize_element<T>(&mut self, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        value.serialize(&mut **self)
    }

    fn end(self) -> Result<()> {
        Ok(())
    }
}

impl<'a> ser::SerializeStruct for &'a mut Serializer {
    type Ok = ();
    type Error = SerializeError;

    fn serialize_field<T>(&mut self, key: &'static str, value: &T) -> Result<()>
    where
        T: ?Sized + Serialize,
    {
        self.push_field(key);
        let ret = value.serialize(&mut **self);
        self.pop_field();
        ret
    }

    fn end(self) -> Result<()> {
        Ok(())
    }
}

pub(super) type Result<T> = std::result::Result<T, SerializeError>;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(super) struct SerializeError(String);

impl ser::Error for SerializeError {
    fn custom<T: ToString>(msg: T) -> Self {
        SerializeError(msg.to_string())
    }
}
