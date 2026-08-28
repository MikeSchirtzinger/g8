# PASS-detect fixture: Python file with G8 magic-comment annotations.
# Accepts both # prefix (Python-native) and // prefix (uniform cross-language).

# @g8.capability(name = "http-handler", status = "in_flight", substrate = "net")
def handle_request(request):
    pass


# @g8.capability(name = "auth-validator", status = "landed", substrate = "auth")
def validate_token(token: str) -> dict:
    return {}


# @g8.convergence_test(for_capability = "http-handler", scenario = "post-request")
def test_post_request():
    pass


# @g8.intent(description = "HTTP request handling", substrate = "net")
_intent_marker = None


# @g8.plan(title = "rate-limiting", status = "Idea", substrate = "net")
def _plan_marker():
    pass
