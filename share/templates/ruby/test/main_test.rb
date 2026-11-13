# frozen_string_literal: true

require 'minitest/autorun'
require_relative '../main'

class MainTest < Minitest::Test
  def test_greeting
    assert_equal 'Hello from {{name}}!', greeting('{{name}}')
  end
end
