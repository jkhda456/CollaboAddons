/* A C extension module, built in the guest the way setuptools builds one (pyext.sh): the
   interpreter's API from its headers, imported from the interpreter when it loads the module. */
#define PY_SSIZE_T_CLEAN
#include <Python.h>
#include <setjmp.h>

static PyObject *SpamError;

static PyObject *spam_add(PyObject *self, PyObject *args) {
  long a, b;
  if (!PyArg_ParseTuple(args, "ll", &a, &b)) return NULL;
  return PyLong_FromLong(a + b);
}

static PyObject *spam_fail(PyObject *self, PyObject *arg) {
  PyErr_Format(SpamError, "failed with %R", arg);
  return NULL;
}

static jmp_buf jb;
static PyObject *spam_jump(PyObject *self, PyObject *noargs) {
  volatile int r = setjmp(jb);
  if (r == 0) longjmp(jb, 3);
  return PyUnicode_FromFormat("jumped %d", r);
}

static PyMethodDef methods[] = {
  {"add", spam_add, METH_VARARGS, "a + b"},
  {"fail", spam_fail, METH_O, "raises spam.error"},
  {"jump", spam_jump, METH_NOARGS, "setjmp/longjmp"},
  {NULL, NULL, 0, NULL},
};

static struct PyModuleDef module = {PyModuleDef_HEAD_INIT, "spam", NULL, -1, methods};

PyMODINIT_FUNC PyInit_spam(void) {
  PyObject *m = PyModule_Create(&module);
  if (!m) return NULL;
  SpamError = PyErr_NewException("spam.error", NULL, NULL);
  PyModule_AddObject(m, "error", SpamError);
  return m;
}
