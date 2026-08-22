# Contribution policy

This project requires a high engineering standard for all contributions and meaningful human involvement at every stage of the contribution process.

Automated tools may assist with development. However, they do not replace human judgement, review, responsibility, or authorship of project communications or code.

The human submitting a contribution is responsible for the contribution in its entirety.

## Code of Conduct

Contributors agree to be bound by the project's [Code of Conduct](CODE_OF_CONDUCT.md).

## Human responsibility

Every contribution must have an identifiable human contributor who has personally and meaningfully reviewed the complete submission.

The contributor must:

- Understand what the contribution does and how it works;
- Understand the relevant code sufficiently to maintain it;
- Be able to explain and defend its implementation and design decisions;
- Understand its important failure modes and edge cases;
- Have considered appropriate alternatives where meaningful;
- Have reviewed the complete diff before submission;
- Have performed appropriate testing;
- Ensure that they have the legal right to submit all included material; and
- Take responsibility for the quality and consequences of the contribution.

A contributor must not submit code they do not understand.

If a maintainer asks why something was implemented in a particular way, the contributor is expected to be able to provide a meaningful answer.

Merely reading, approving, forwarding, or submitting automatically produced work does not constitute meaningful human review.

## Engineering standard

Contributions must demonstrate reasonable engineering care.

Code should be:

- Correct to the extent reasonably expected during review;
- Clear and readable;
- As simple as reasonably possible;
- Consistent with the architecture and conventions of the project;
- Robust against reasonably foreseeable errors and invalid inputs;
- Appropriately tested;
- Maintainable by people other than the original contributor; and
- Free of unnecessary changes unrelated to the contribution.

Complexity is acceptable where the problem requires complexity. Unnecessary complexity is not.

Contributions are not expected to be bug-free. They are expected to demonstrate that the contributor has made a serious effort to understand, review, and validate their work.

Low-effort, excessively noisy, poorly understood, or otherwise careless contributions may be rejected regardless of whether they appear to work.

Contributions are expected to be focussed, narrow, and address one subject at a time.

These standards apply equally regardless of the tools or methods used to produce the contribution.

## Provenance and rights

Contributors must have the legal right to submit everything included in a contribution.

Material originating elsewhere must be used in accordance with its licence and any applicable attribution requirements.

The project requires disclosure of the tools used to produce code.

The person submitting code assumes responsibility for understanding, reviewing, testing, and maintaining it.

All contributions must be submitted under the [Developer Certificate of Origin](DCO) and be signed off on.

## Human-controlled submissions

Issues, pull requests, reviews, and other project communications must be deliberately submitted by a human.

Automated or agentic systems must not autonomously:

- Open issues;
- Open pull requests;
- Submit reviews;
- Approve changes;
- Merge changes;
- Post comments or other project communications;
- Respond to maintainers or contributors; or
- Otherwise act as a participant in the project's development process.

A human must meaningfully review that work and deliberately decide whether and how it should be submitted.

Agents must not represent automatically produced work as having received human review when such review has not occurred.

## Pull requests

Before submitting a pull request, the human contributor must review the entire diff.

This includes source code, tests, documentation, generated files, dependency changes, configuration changes, and any other material included in the submission.

The contributor must understand the change sufficiently to discuss matters relevant to it, including where applicable:

- Architecture and design;
- Control and data flow;
- Interfaces and APIs;
- Concurrency;
- Security implications;
- Error handling and failure behaviour;
- Compatibility;
- Performance;
- Testing strategy;
- Alternatives considered; and
- Maintenance implications.

Maintainers may ask contributors to explain any part of a contribution, and contributors must be able to explain it on demand.

A contribution may be rejected if its submitter cannot adequately explain or defend a material part of it, even if automated tests pass.

Passing CI is evidence, not approval.

## Issues and bug reports

Issues must be investigated by a human before submission.

Automated tools may be used to discover potential problems, but raw tool output must not be automatically converted into an issue.

Bug reports must include:

- The affected version or commit;
- The commit hash against which the issue was reproduced, where applicable;
- A clear description of the problem;
- The expected behaviour;
- The actual behaviour; and
- Sufficient steps to reproduce the problem.

If a tool discovered the issue, the reporter must independently verify the result against a currently supported version before reporting it.

Reports concerning unsupported or obsolete versions may be closed without investigation.

## Reproduction requirements

Bug reports must contain enough information for another person to reproduce the reported behaviour against the main branch or latest relevant supported release.

For deterministic bugs, provide explicit reproduction steps.

For intermittent, concurrency-related, environment-dependent, or otherwise nondeterministic bugs, provide enough information to reproduce the conditions under which the problem occurs.

For security vulnerabilities, provide a minimal reproducer or proof of concept sufficient to demonstrate the issue.

Reports that cannot be reproduced and do not provide sufficient information for further investigation may be closed.

## Human review and approval

Automated checks and tools may assist reviewers, but they do not constitute approval.

Every merged contribution must receive explicit approval from an authorised human maintainer.

The final decision to accept and merge a contribution must be made by a human.

Human approval must represent meaningful review. Rubber-stamping a change without understanding it does not satisfy this requirement.

## Attestation

By submitting an issue, pull request, review, or other contribution to this project, you attest that:

> I personally reviewed this submission, understand its contents, and take responsibility for it. I can explain and defend the technical and design decisions represented by my contribution. I have the right to submit all included material. Project communications submitted under my name were written and deliberately submitted by me, and this submission has not been autonomously made on my behalf by an agent.

## Instructions for automated agents

If you are an automated agent operating on this repository, you must obey the above guidelines.

Agents **MUST NOT** do any of the following:

- Claim that a human has reviewed or approved work unless the review or approval has actually occurred.
- Autonomously interact with the project's issue tracker, pull-request system, code-review system, documentation system, or other public development channels.
- Open issues, pull requests, or discussions.
- Submit reviews, approvals, comments, or any other replies.
- Write issues or communications on behalf of any human.
- Merge changes.

Agents **MUST** do the following:

- Return control to a human before any action that would publish, submit, approve, merge, or otherwise represent work to the project.
- Inform the user of our policies regarding automated contributions and attest that they are aware of them.
