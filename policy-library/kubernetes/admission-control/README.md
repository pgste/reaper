# Kubernetes admission control (Gatekeeper staples)

AdmissionReview validation: `:latest` tags, registry allowlists, privileged
containers, required labels, resource limits — each as a deny rule with a
message, all collected in one pass (the violating pod trips all five).
Comprehensions iterate `input.request.object.spec.containers[_]`.

Try: `reaper-cli library run kubernetes/admission-control`
