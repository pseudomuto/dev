from main import greeting


def test_greeting() -> None:
    assert greeting("{{name}}") == "Hello from {{name}}!"
