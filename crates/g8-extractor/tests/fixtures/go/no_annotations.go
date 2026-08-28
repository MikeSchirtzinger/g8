// NO-detect fixture: Go file with no G8 annotations.

package example

// A regular Go comment.
// @some_other_tag — not G8

func RegularFunction(x int) int {
	return x + 1
}

// Another comment.
type RegularType struct{}
