; HTML highlights (compatible with tree-sitter-html 0.23)

(tag_name) @tag
(attribute_name) @attribute
(attribute_value) @string
(quoted_attribute_value) @string

(comment) @comment
(doctype) @keyword

["<" ">" "</" "/>" "="] @punctuation
