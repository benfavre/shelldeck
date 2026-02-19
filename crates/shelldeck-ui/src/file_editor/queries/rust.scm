; Rust highlights (compatible with tree-sitter-rust 0.24)

(type_identifier) @type
(primitive_type) @type
(field_identifier) @property

(call_expression
  function: (identifier) @function)
(call_expression
  function: (field_expression
    field: (field_identifier) @function))
(call_expression
  function: (scoped_identifier
    "::"
    name: (identifier) @function))

(generic_function
  function: (identifier) @function)

(macro_invocation
  macro: (identifier) @function
  "!" @function)

(function_item (identifier) @function)
(function_signature_item (identifier) @function)

(line_comment) @comment
(block_comment) @comment

"(" @punctuation
")" @punctuation
"[" @punctuation
"]" @punctuation
"{" @punctuation
"}" @punctuation

"::" @punctuation
":" @punctuation
"." @punctuation
"," @punctuation
";" @punctuation

(parameter (identifier) @variable)
(lifetime (identifier) @label)

"as" @keyword
"async" @keyword
"await" @keyword
"break" @keyword
"const" @keyword
"continue" @keyword
"default" @keyword
"dyn" @keyword
"else" @keyword
"enum" @keyword
"extern" @keyword
"fn" @keyword
"for" @keyword
"if" @keyword
"impl" @keyword
"in" @keyword
"let" @keyword
"loop" @keyword
"macro_rules!" @keyword
"match" @keyword
"mod" @keyword
"move" @keyword
"pub" @keyword
"ref" @keyword
"return" @keyword
"static" @keyword
"struct" @keyword
"trait" @keyword
"type" @keyword
"unsafe" @keyword
"use" @keyword
"where" @keyword
"while" @keyword
"yield" @keyword
(crate) @keyword
(mutable_specifier) @keyword
(super) @keyword
(self) @variable

(char_literal) @string
(string_literal) @string
(raw_string_literal) @string

(boolean_literal) @constant
(integer_literal) @number
(float_literal) @number

(escape_sequence) @escape

(attribute_item) @attribute
(inner_attribute_item) @attribute

"*" @operator
"&" @operator
"'" @operator
