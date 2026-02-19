; YAML highlights (compatible with tree-sitter-yaml 0.7)

(block_mapping_pair
  key: (flow_node) @property)
(flow_pair
  key: (flow_node) @property)

(string_scalar) @string
(double_quote_scalar) @string
(single_quote_scalar) @string
(block_scalar) @string

(integer_scalar) @number
(float_scalar) @number
(boolean_scalar) @constant
(null_scalar) @constant

(comment) @comment
(anchor) @label
(alias) @label
(tag) @attribute

[":" "-" "," "[" "]" "{" "}"] @punctuation
