power_value = 2^10
shifted_power = (1 + 3)^2
literal_formula = "x^2 + 1 should stay unchanged in strings"
triple_literal = """
caret ^ should also stay inside triple-quoted text
"""
comment_sample = 7  # caret ^ in comments stays literal


def local_power_report():
    return power_value + shifted_power
