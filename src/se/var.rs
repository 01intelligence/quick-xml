use crate::{
    errors::{serialize::DeError, Error},
    events::{BytesEnd, BytesStart, Event},
    se::Serializer,
    writer::Writer,
};
use serde::ser::{self, Serialize};
use std::io::Write;

/// An implementation of `SerializeMap` for serializing to XML.
pub struct Map<'r, 'w, W>
where
    W: 'w + Write,
{
    parent: &'w mut Serializer<'r, W>,
}

impl<'r, 'w, W> Map<'r, 'w, W>
where
    W: 'w + Write,
{
    /// Create a new Map
    pub fn new(parent: &'w mut Serializer<'r, W>) -> Self {
        Map { parent }
    }
}

impl<'r, 'w, W> ser::SerializeMap for Map<'r, 'w, W>
where
    W: 'w + Write,
{
    type Ok = ();
    type Error = DeError;

    fn serialize_key<T: ?Sized + Serialize>(&mut self, _: &T) -> Result<(), DeError> {
        Err(DeError::Unsupported(
            "impossible to serialize the key on its own, please use serialize_entry()",
        ))
    }

    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<(), DeError> {
        value.serialize(&mut *self.parent)
    }

    fn end(self) -> Result<Self::Ok, DeError> {
        if let Some(tag) = self.parent.root_tag {
            self.parent
                .writer
                .write_event(Event::End(BytesEnd::borrowed(tag.as_bytes())))?;
        }
        Ok(())
    }

    fn serialize_entry<K: ?Sized + Serialize, V: ?Sized + Serialize>(
        &mut self,
        key: &K,
        value: &V,
    ) -> Result<(), DeError> {
        let mut buffer = Vec::new();
        let mut writer = Writer::new(&mut buffer);
        if let Some(indent) = &self.parent.writer.indent {
            writer.indent = Some(indent.clone());
        }
        let mut serializer = Serializer::with_root(writer, None);
        key.serialize(&mut serializer)?;

        let tag = BytesStart::borrowed_name(&buffer);
        self.parent
            .writer
            .write_event(Event::Start(tag.to_borrowed()))?;

        let root = self.parent.root_tag.take();
        value.serialize(&mut *self.parent)?;
        self.parent.root_tag = root;

        self.parent
            .writer
            .write_event(Event::End(tag.to_end()))?;
        Ok(())
    }
}

/// An implementation of `SerializeStruct` for serializing to XML.
pub struct Struct<'r, 'w, W>
where
    W: 'w + Write,
{
    parent: &'w mut Serializer<'r, W>,
    /// Buffer for holding fields, serialized as attributes. Doesn't allocate
    /// if there are no fields represented as attributes
    attrs: Option<BytesStart<'w>>,
    /// Buffer for holding fields, serialized as elements
    children: Vec<u8>,
    /// Buffer for serializing one field. Cleared after serialize each field
    buffer: Vec<u8>,
    begun: bool,
}

impl<'r, 'w, W> Struct<'r, 'w, W>
where
    W: 'w + Write,
{
    /// Create a new `Struct`
    pub fn new(parent: &'w mut Serializer<'r, W>, name: Option<&'r str>) -> Self {
        Struct {
            parent,
            attrs: name.map(|name| BytesStart::borrowed_name(name.as_bytes())),
            children: Vec::new(),
            buffer: Vec::new(),
            begun: false,
        }
    }
}

impl<'r, 'w, W> ser::SerializeStruct for Struct<'r, 'w, W>
where
    W: 'w + Write,
{
    type Ok = ();
    type Error = DeError;

    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), DeError> {
        if !self.begun {
            self.begun = true;
            self.parent.writer.write_event(Event::IndentGlow)?;
        }

        let (key, is_attr, is_name) = match key.strip_suffix(" $attr") {
            Some(key) => (key, true, false),
            None => {
                match key.strip_suffix(" $name") {
                    Some(key) => (key, false, true),
                    None => (key, false, false)
                }
            },
        };

        if is_name {
            if let Some(attrs) = &mut self.attrs {
                attrs.set_name(key.as_bytes());
            } else {
                self.attrs = Some(BytesStart::borrowed_name(key.as_bytes()))
            }
            return Ok(());
        }

        // TODO: Inherit indentation state from self.parent.writer
        let mut writer = Writer::new(&mut self.buffer);
        if let Some(indent) = &self.parent.writer.indent {
            writer.indent = Some(indent.clone());
        }
        let mut serializer = Serializer::with_root(writer, if !is_attr { Some(key) } else { None });
        value.serialize(&mut serializer)?;

        if !self.buffer.is_empty() {
            if !is_attr {
                // Drains buffer, moves it to children
                self.children.append(&mut self.buffer);
            } else if let Some(attrs) = &mut self.attrs {
                attrs.push_attribute((key.as_bytes(), self.buffer.as_ref()));
                self.buffer.clear();
            }
        }

        Ok(())
    }

    fn end(self) -> Result<Self::Ok, DeError> {
        self.parent.writer.write_event(Event::IndentShrink)?;

        if self.children.is_empty() {
            if let Some(attrs) = self.attrs {
                self.parent.writer.write_event(Event::Empty(attrs))?;
            }
        } else {
            if let Some(attrs) = &self.attrs {
                self.parent
                    .writer
                    .write_event(Event::Start(attrs.to_borrowed()))?;
            }
            self.parent.writer.write(&self.children)?;
            if let Some(attrs) = &self.attrs {
                self.parent
                    .writer
                    .write_event(Event::End(attrs.to_end()))?;
            }
        }
        Ok(())
    }
}

impl<'r, 'w, W> ser::SerializeStructVariant for Struct<'r, 'w, W>
where
    W: 'w + Write,
{
    type Ok = ();
    type Error = DeError;

    #[inline]
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Self::Error> {
        <Self as ser::SerializeStruct>::serialize_field(self, key, value)
    }

    #[inline]
    fn end(self) -> Result<Self::Ok, Self::Error> {
        <Self as ser::SerializeStruct>::end(self)
    }
}

/// An implementation of `SerializeSeq' for serializing to XML.
pub struct Seq<'r, 'w, W>
where
    W: 'w + Write,
{
    parent: &'w mut Serializer<'r, W>,
}

impl<'r, 'w, W> Seq<'r, 'w, W>
where
    W: 'w + Write,
{
    /// Create a new `Seq`
    pub fn new(parent: &'w mut Serializer<'r, W>) -> Self {
        Seq { parent }
    }
}

impl<'r, 'w, W> ser::SerializeSeq for Seq<'r, 'w, W>
where
    W: 'w + Write,
{
    type Ok = ();
    type Error = DeError;

    fn serialize_element<T: ?Sized>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        value.serialize(&mut *self.parent)?;
        Ok(())
    }

    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

/// An implementation of `SerializeTuple`, `SerializeTupleStruct` and
/// `SerializeTupleVariant` for serializing to XML.
pub struct Tuple<'r, 'w, W>
where
    W: 'w + Write,
{
    parent: &'w mut Serializer<'r, W>,
    /// Possible qualified name of XML tag surrounding each element
    name: &'r str,
}

impl<'r, 'w, W> Tuple<'r, 'w, W>
where
    W: 'w + Write,
{
    /// Create a new `Tuple`
    pub fn new(parent: &'w mut Serializer<'r, W>, name: &'r str) -> Self {
        Tuple { parent, name }
    }
}

impl<'r, 'w, W> ser::SerializeTuple for Tuple<'r, 'w, W>
where
    W: 'w + Write,
{
    type Ok = ();
    type Error = DeError;

    fn serialize_element<T: ?Sized>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        let root = self.parent.root_tag.replace(self.name);
        value.serialize(&mut *self.parent)?;
        self.parent.root_tag = root;
        Ok(())
    }

    #[inline]
    fn end(self) -> Result<Self::Ok, Self::Error> {
        Ok(())
    }
}

impl<'r, 'w, W> ser::SerializeTupleStruct for Tuple<'r, 'w, W>
where
    W: 'w + Write,
{
    type Ok = ();
    type Error = DeError;

    #[inline]
    fn serialize_field<T: ?Sized>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        <Self as ser::SerializeTuple>::serialize_element(self, value)
    }

    #[inline]
    fn end(self) -> Result<Self::Ok, Self::Error> {
        <Self as ser::SerializeTuple>::end(self)
    }
}

impl<'r, 'w, W> ser::SerializeTupleVariant for Tuple<'r, 'w, W>
where
    W: 'w + Write,
{
    type Ok = ();
    type Error = DeError;

    #[inline]
    fn serialize_field<T: ?Sized>(&mut self, value: &T) -> Result<(), Self::Error>
    where
        T: Serialize,
    {
        <Self as ser::SerializeTuple>::serialize_element(self, value)
    }

    #[inline]
    fn end(self) -> Result<Self::Ok, Self::Error> {
        <Self as ser::SerializeTuple>::end(self)
    }
}
