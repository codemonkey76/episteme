import 'package:flutter/material.dart';
import 'package:provider/provider.dart';

import '../api/models.dart';
import '../main.dart';
import '../state/shipments.dart';
import 'shipment_screen.dart';

/// What's on the way — the mobile counterpart of the Shipments window.
class ShipmentsTab extends StatefulWidget {
  const ShipmentsTab({super.key});

  @override
  State<ShipmentsTab> createState() => _ShipmentsTabState();
}

class _ShipmentsTabState extends State<ShipmentsTab> {
  static const _filters = [
    ('active', 'On the way'),
    ('delivered', 'Delivered'),
    ('all', 'All'),
  ];

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      context.read<ShipmentsStore>().load();
    });
  }

  @override
  Widget build(BuildContext context) {
    final store = context.watch<ShipmentsStore>();

    return Scaffold(
      backgroundColor: Palette.bg,
      floatingActionButton: FloatingActionButton(
        onPressed: () => showShipmentEditor(context),
        child: const Icon(Icons.add),
      ),
      body: Column(
        children: [
          SingleChildScrollView(
            scrollDirection: Axis.horizontal,
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 4),
            child: Row(
              children: _filters
                  .map((f) => Padding(
                        padding: const EdgeInsets.only(right: 8),
                        child: ChoiceChip(
                          label: Text(f.$2),
                          selected: store.filter == f.$1,
                          onSelected: (_) => store.setFilter(f.$1),
                        ),
                      ))
                  .toList(),
            ),
          ),
          if (store.error != null)
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 4),
              child: Text(store.error!,
                  style: const TextStyle(color: Palette.danger, fontSize: 12.5)),
            ),
          Expanded(
            child: RefreshIndicator(
              onRefresh: store.load,
              child: store.loading && store.shipments.isEmpty
                  ? const Center(child: CircularProgressIndicator())
                  : store.shipments.isEmpty
                      ? ListView(
                          physics: const AlwaysScrollableScrollPhysics(),
                          children: const [
                            Padding(
                              padding: EdgeInsets.only(top: 120, left: 32, right: 32),
                              child: Center(
                                child: Text(
                                  'Nothing on the way. Add a parcel, or let the '
                                  'email auto-sort pick up your next shipping notice.',
                                  textAlign: TextAlign.center,
                                  style: TextStyle(color: Palette.faint),
                                ),
                              ),
                            ),
                          ],
                        )
                      : ListView.builder(
                          physics: const AlwaysScrollableScrollPhysics(),
                          padding: const EdgeInsets.only(bottom: 88),
                          itemCount: store.shipments.length,
                          itemBuilder: (_, i) =>
                              _ShipmentTile(shipment: store.shipments[i]),
                        ),
            ),
          ),
        ],
      ),
    );
  }
}

class _ShipmentTile extends StatelessWidget {
  const _ShipmentTile({required this.shipment});
  final Shipment shipment;

  @override
  Widget build(BuildContext context) {
    final store = context.read<ShipmentsStore>();
    final subtitle =
        [shipment.merchant, shipment.carrier].whereType<String>().join(' · ');

    return ListTile(
      onTap: () => Navigator.of(context).push(MaterialPageRoute(
          builder: (_) => ShipmentScreen(shipmentId: shipment.id))),
      leading: Container(
        width: 44,
        height: 44,
        decoration: BoxDecoration(
          color: Palette.surface,
          border: Border.all(color: Palette.raised),
          borderRadius: BorderRadius.circular(8),
        ),
        clipBehavior: Clip.antiAlias,
        child: shipment.hasPhoto
            ? Image.network(
                store.photoUrl(shipment),
                headers: store.photoHeaders,
                fit: BoxFit.cover,
                errorBuilder: (_, _, _) => const Icon(
                    Icons.inventory_2_outlined, color: Palette.faint, size: 18),
              )
            : const Icon(Icons.inventory_2_outlined,
                color: Palette.faint, size: 18),
      ),
      title: Text(shipment.label,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: const TextStyle(
              color: Palette.fg, fontSize: 15, fontWeight: FontWeight.w500)),
      subtitle: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (subtitle.isNotEmpty)
            Text(subtitle,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(color: Palette.muted, fontSize: 12.5)),
          Padding(
            padding: const EdgeInsets.only(top: 2),
            child: Text(
              shipment.isLate
                  ? 'Overdue — ${shipmentEtaText(shipment.eta)}'
                  : shipmentEtaText(shipment.eta),
              style: TextStyle(
                  color: shipment.isLate ? Palette.warn : Palette.faint,
                  fontSize: 11.5),
            ),
          ),
        ],
      ),
      trailing: Container(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
        decoration: BoxDecoration(
          color: shipmentStatusColor(shipment.status).withValues(alpha: 0.14),
          borderRadius: BorderRadius.circular(999),
        ),
        child: Text(shipment.statusLabel,
            style: TextStyle(
                color: shipmentStatusColor(shipment.status), fontSize: 11)),
      ),
    );
  }
}
