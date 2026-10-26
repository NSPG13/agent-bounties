def get_direct_bounty_evidence_checklist():
    """
    Provides a concise checklist for submitting evidence for a direct bounty.

    This checklist guides bounty hunters on the essential items to include
    when presenting their completed work for verification and reward.

    Returns:
        list[str]: A list of strings, each representing an evidence item.
    """
    checklist = [
        "Link to the Pull Request (PR) or specific commit(s) containing the solution.",
        "A clear, step-by-step description of how to verify the solution (e.g., setup, commands to run, expected output).",
        "Evidence of successful verification (e.g., console output, screenshots, video demonstration, test reports).",
        "Any relevant documentation updates (e.g., README, inline code comments, design notes).",
        "A brief summary of the implemented solution and how it directly addresses the bounty requirements."
    ]
    return checklist

if __name__ == "__main__":
    print("--- Direct Bounty Evidence Checklist ---")
    for i, item in enumerate(get_direct_bounty_evidence_checklist(), 1):
        print(f"{i}. {item}")
    print("----------------------------------------")