# Kubernetes Admission Webhook Deployment

The Reaper Agent is a direct `ValidatingWebhookConfiguration` target: it
speaks native `AdmissionReview` (admission.k8s.io/**v1**) on
`POST /api/v1/admission/{policy}` — no adapter or sidecar shim between the
API server and the agent. Validation runs on the compiled check driver
(R4-01 Phase B.3), so a full admission decision on the k8s library policy is
~14 µs of evaluation, not a per-call policy parse.

## Endpoint contract

```
POST /api/v1/admission/{policy}
Content-Type: application/json
Body: AdmissionReview (admission.k8s.io/v1)
```

`{policy}` names a **deployed** policy on the agent (the API server cannot
put a policy name in the request body, and Kubernetes forbids query
parameters in webhook URLs — the path carries the selection).

Mapping into the policy request:

| AdmissionReview field | Policy view |
|---|---|
| whole review body | `input` (policies read `input.request.object...` — the OPA convention) |
| `request.operation` | `action`, lowercased (`create`, `update`, `delete`, `connect`) |
| `request.resource.resource` (else `request.kind.kind`) | `resource`, lowercased (e.g. `pods`) |
| `request.userInfo.username` | `principal` (request context) |
| `request.namespace` | `namespace` (request context) |

Response: a well-formed v1 `AdmissionReview` with `response.uid` echoed,
`response.allowed`, and — on denial — every matching deny rule's rendered
`with message` text joined into `response.status.message` (`status.code`
403), which is what `kubectl` shows the user:

```json
{
  "apiVersion": "admission.k8s.io/v1",
  "kind": "AdmissionReview",
  "response": {
    "uid": "705ab4f5-6393-11e8-b7cc-42010a800002",
    "allowed": false,
    "status": {
      "code": 403,
      "message": "image uses :latest tag: registry.corp.internal/web:latest; privileged container: web"
    }
  }
}
```

## Failure posture — the webhook itself fails closed

| Situation | Answer |
|---|---|
| Violations found | 200, `allowed: false`, messages in `status.message`, code 403 |
| No violations | 200, `allowed: true`, no `status` |
| Named policy not deployed / not a DSL policy / evaluation error | 200, **`allowed: false`**, reason in `status.message`, code 500 |
| Body has no `request.uid`, or `apiVersion` is not `admission.k8s.io/v1` | 400 (no well-formed response exists without a uid) |
| Agent unreachable / TLS failure / timeout | no response — the webhook's **`failurePolicy`** decides |

The third row is deliberate: configuration mistakes (typo'd policy name,
policy not yet synced) DENY admission with an operator-readable reason
rather than silently admitting workloads, independent of `failurePolicy`.

### failurePolicy guidance

Set **`failurePolicy: Fail`** (fail closed). Reaper's own evaluation is
total — every in-band problem already answers `allowed: false` — so
`failurePolicy` only governs transport-level failure (agent down, network,
TLS). For an admission *guardrail* policy, admitting workloads unvalidated
during an outage defeats the control; run ≥2 agent replicas behind the
Service instead of weakening the policy to `Ignore`. If you do accept
availability over enforcement for a low-stakes policy (e.g. label hygiene),
`failurePolicy: Ignore` plus alerting on webhook errors is the explicit
trade — make it per-webhook, not the default. Scope the webhook with
`namespaceSelector`/`objectSelector` so a cluster-wide outage cannot brick
`kube-system` (see the manifest below), and keep `timeoutSeconds` low
(1–2 s is generous: evaluation is microseconds, so the budget is purely
network).

## TLS — required by the API server

Kubernetes only calls webhooks over HTTPS. The agent terminates TLS
natively:

```bash
REAPER_TLS_ENABLED=true
REAPER_TLS_CERT=/certs/tls.crt      # server cert, SAN must cover the Service DNS name
REAPER_TLS_KEY=/certs/tls.key
```

The serving certificate's SAN must include
`<service>.<namespace>.svc` (e.g. `reaper-agent.reaper-system.svc`) — the
API server connects by that name. Issue it with cert-manager or your PKI
and mount it into the agent pod; put the issuing CA (or the cert itself if
self-signed) in the webhook's `clientConfig.caBundle`.

Mutual TLS: the agent can additionally demand a client certificate
(`REAPER_TLS_CA=/certs/ca.crt`, `REAPER_TLS_REQUIRE_CLIENT_CERT=true`).
The Kubernetes API server presents one only when the cluster is configured
with an admission `AdmissionConfiguration` kubeconfig for the webhook; if
you don't control apiserver flags, leave client-cert verification off for
this route. The admission and check routes are data-plane (read-only
evaluation, no policy/data mutation), so they follow the agent's data-plane
auth posture — the management routes stay fully gated either way.

## Example manifests

Webhook (validating pods against the library policy `k8s_admission`):

```yaml
apiVersion: admissionregistration.k8s.io/v1
kind: ValidatingWebhookConfiguration
metadata:
  name: reaper-admission
webhooks:
  - name: pods.reaper.example.com
    admissionReviewVersions: ["v1"]
    sideEffects: None
    failurePolicy: Fail          # fail closed (see guidance above)
    timeoutSeconds: 2
    clientConfig:
      service:
        name: reaper-agent
        namespace: reaper-system
        path: /api/v1/admission/k8s_admission
        port: 8443
      caBundle: <base64 CA cert>
    rules:
      - apiGroups: [""]
        apiVersions: ["v1"]
        operations: ["CREATE", "UPDATE"]
        resources: ["pods"]
    namespaceSelector:
      matchExpressions:
        - key: kubernetes.io/metadata.name
          operator: NotIn
          values: ["kube-system", "reaper-system"]
```

Agent Service (TLS port):

```yaml
apiVersion: v1
kind: Service
metadata:
  name: reaper-agent
  namespace: reaper-system
spec:
  selector:
    app: reaper-agent
  ports:
    - name: https
      port: 8443
      targetPort: 8080   # agent serves TLS itself when REAPER_TLS_ENABLED=true
```

## Smoke test

With the policy deployed and a fixture on hand
(`services/reaper-agent/tests/fixtures/admission/pod-violating.json`):

```bash
curl -sk https://reaper-agent.reaper-system.svc:8443/api/v1/admission/k8s_admission \
  -H 'Content-Type: application/json' \
  -d @pod-violating.json | jq .response
```

Expect `allowed: false` with all five library-rule messages in
`status.message`. The generic document-check endpoint
(`POST /api/v1/check`, JSON body `{"policy_name": ..., "input": ...}`)
serves the same driver and returns the structured violation list — use it
from CI or for debugging what the webhook will decide.
