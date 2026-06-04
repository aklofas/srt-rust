package org.tstrans;

/** Base for all checked exceptions thrown across the JNI boundary (spec §5.3). */
public abstract class BindingException extends Exception {
    private static final long serialVersionUID = 1L;

    protected BindingException(String message) {
        super(message);
    }
}
