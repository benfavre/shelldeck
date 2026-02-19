; TOML highlights (compatible with tree-sitter-toml-ng 0.7)

(bare_key) @property
(dotted_key) @property
(quoted_key) @property

(string) @string
(integer) @number
(float) @number
(boolean) @constant

(offset_date_time) @string
(local_date_time) @string
(local_date) @string
(local_time) @string

(comment) @comment

["=" ","] @operator
["[" "]" "[[" "]]"] @punctuation
