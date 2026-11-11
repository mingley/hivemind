import logging

log = logging.getLogger(__name__)


def notify_refund(order_id: str) -> None:
    log.info("refund notified for %s", order_id)
