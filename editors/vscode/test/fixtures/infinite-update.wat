(module
  (memory (export "memory") 1)
  (func (export "update")
    (loop $forever
      br $forever)))
