// PASS-detect fixture: Go file with G8 magic-comment annotations.

package example

// @g8.capability(name = "streaming-export", status = "in_flight", substrate = "export")
func ExportStream(w interface{}) error {
	return nil
}

// @g8.capability(name = "batch-export", status = "proposed", substrate = "export")
func ExportBatch(jobs []string) ([]byte, error) {
	return nil, nil
}

// @g8.convergence_test(for_capability = "streaming-export", scenario = "large-payload")
func TestStreamingExportLargePayload(t interface{}) {}

// @g8.intent(description = "Export pipeline functions", substrate = "export")
var _ = nil

// @g8.plan(title = "async-export", status = "Idea", substrate = "export")
func planAsyncExport() {}
