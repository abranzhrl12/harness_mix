# Go native support

Go files (`.go`) use the bundled tree-sitter grammar for syntax validation,
symbol outline/read, and local CodeGraph extraction. Validation requires a Go
`package` clause and checks syntax only; it does not run `go build` or type
checking. The parser caps a source file at 512 KiB. The existing Go verifier
remains the build and test path.

Symbol navigation includes package functions, receiver methods, named types,
type aliases, structs, interfaces, interface method elements, struct fields,
and imports.
Reads return exact source spans, and symbol IDs use UTF-16 positions while
parser ranges remain UTF-8 byte offsets.

The parser accepts at most 512 KiB per source file. Symbol traversal is bounded
to 2,000 symbols, 100,000 visited nodes and 128 nesting levels. It skips function
bodies when collecting declarations. Embedded struct fields are not exposed;
grouped field declarations currently expose their first field name.

The Go graph records file containment, declarations, and explicit import
declarations. It does not resolve imported package names, cross-package or
same-name references, function calls, dynamic calls, method dispatch, or
interface implementations. These require package/type information that the
syntax tree alone does not establish.
