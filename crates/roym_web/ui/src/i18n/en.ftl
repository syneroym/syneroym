# Hub text in English. This file is the fallback for every language: a
# message that another language lacks is shown from here.
#
# To add a message: write `id = text` on one line (or an indented block),
# add the same id to every other .ftl file, and call `t("id")` with the id
# as a literal. A message never starts or ends with a space; Fluent removes
# it. Pass codes and ids as strings, because a number argument is shown
# with digit grouping.

language-picker-label = Language
language-not-saved = This browser does not let the Hub save the language, so it stays as it is.

age-moments = moments ago

money-input-invalid = "{ $input }" is not an amount like { $example } for this currency

# Errors. `$detail` is the service's own English text.
error-service-unavailable = { $detail }
error-service-timeout = { $detail }
error-permission = { $detail }
error-quota = { $detail }
error-not-signed-in = { $detail }
error-not-owner = { $detail }
error-no-owner = { $detail }
error-not-local = this installation refused a request that did not come from you
error-method-not-found = { $detail }
error-invalid-input = { $detail }
error-internal = { $detail }
error-service = { $detail }
error-network = Could not reach this installation: { $detail }
error-http = HTTP error { $code }
