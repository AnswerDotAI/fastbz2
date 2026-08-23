pub fn hello(name: &str) -> String {
    format!("Hello, {name}!")
}

#[cfg(feature = "python")]
mod python {
    use super::hello;
    use pyo3::prelude::*;

    #[pyfunction(name = "hello")]
    fn py_hello(name: &str) -> String {
        hello(name)
    }

    #[pymodule]
    fn _core(m: &Bound<'_, PyModule>) -> PyResult<()> {
        m.add_function(wrap_pyfunction!(py_hello, m)?)?;
        m.add("__version__", env!("CARGO_PKG_VERSION"))?;
        Ok(())
    }
}
