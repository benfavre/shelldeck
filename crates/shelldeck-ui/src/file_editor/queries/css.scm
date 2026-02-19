; CSS highlights (compatible with tree-sitter-css 0.25)

(tag_name) @tag
(class_name) @type
(id_name) @constant
(property_name) @property
(feature_name) @property

(string_value) @string
(color_value) @number
(integer_value) @number
(float_value) @number
(plain_value) @variable

(comment) @comment

(pseudo_class_selector
  (class_name) @attribute)
(pseudo_element_selector
  "::" @punctuation)

["{" "}" "(" ")" "[" "]"] @punctuation
[":" ";" "," "." "#" ">" "+" "~"] @punctuation
