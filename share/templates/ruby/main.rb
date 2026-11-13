# frozen_string_literal: true

def greeting(name)
  "Hello from #{name}!"
end

puts greeting('{{name}}') if __FILE__ == $PROGRAM_NAME
